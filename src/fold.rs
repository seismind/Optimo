use crate::aggregate_state::AggregateState;
use crate::operational_policy::map_severity;
use crate::ocrys::types::{OCRDocument, OCRLine, OCRPage};
use crate::profile::IngestionProfile;
use crate::snapshot::{ReducerSnapshot, SnapshotMetadata};
use crate::observation::{OcrObservation, ObservationStatus};
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

// TODO: snapshot persistence
// TODO: event replay integration
// TODO: distributed reducer partitioning

const SIM_THRESHOLD: f64 = 0.90;
const SCORE_SCALE: u32 = 10_000;
const WEAK_MARGIN_BPS: u32 = 750;

type Position = (u32, u32);

#[derive(Debug, Clone)]
struct FoldItem {
    position: Position,
    text: String,
    weight: f32,
    source: String,
}

#[derive(Debug, Clone)]
struct CandidateVote {
    text: String,
    total_weight: f32,
    count: u32,
}

#[derive(Debug, Clone)]
struct VoteCluster {
    text: String,
    total_weight: f32,
    count: u32,
}

#[derive(Debug, Clone, Default)]
struct PositionAccumulator {
    clusters: Vec<VoteCluster>,
    winner_text: String,
    convergence_score_bps: u32,
    ambiguity_score_bps: u32,
}

impl PositionAccumulator {
    fn add_vote(&mut self, text: String, weight: f32) {
        let cluster_idx = self.find_cluster_index(&text);
        match cluster_idx {
            Some(idx) => {
                let cluster = &mut self.clusters[idx];
                cluster.total_weight += weight;
                cluster.count = cluster.count.saturating_add(1);
                cluster.text = prefer_text(&cluster.text, &text).to_string();
            }
            None => self.clusters.push(VoteCluster {
                text,
                total_weight: weight,
                count: 1,
            }),
        }

        self.recompute_winner();
    }

    fn find_cluster_index(&self, text: &str) -> Option<usize> {
        let mut best_idx: Option<usize> = None;
        let mut best_sim = f64::MIN;

        for (idx, cluster) in self.clusters.iter().enumerate() {
            let sim = strsim::jaro_winkler(&cluster.text, text);
            if sim < SIM_THRESHOLD {
                continue;
            }

            if sim > best_sim {
                best_sim = sim;
                best_idx = Some(idx);
            }
        }

        best_idx
    }

    fn alternatives(&self) -> Vec<CandidateVote> {
        self.clusters
            .iter()
            .map(|c| CandidateVote {
                text: c.text.clone(),
                total_weight: c.total_weight,
                count: c.count,
            })
            .collect()
    }

    fn recompute_winner(&mut self) {
        self.clusters.sort_by(|a, b| {
            b.total_weight
                .total_cmp(&a.total_weight)
                .then_with(|| b.count.cmp(&a.count))
                .then_with(|| a.text.cmp(&b.text))
        });

        let alternatives = self.alternatives();
        let winner = alternatives.first().cloned().unwrap_or(CandidateVote {
            text: String::new(),
            total_weight: 0.0,
            count: 0,
        });
        let runner_up_weight = alternatives.get(1).map(|x| x.total_weight).unwrap_or(0.0);
        let total_weight: f32 = alternatives.iter().map(|x| x.total_weight).sum();

        self.winner_text = winner.text.clone();
        self.convergence_score_bps = to_bps(winner.total_weight, total_weight);
        self.ambiguity_score_bps = to_bps(runner_up_weight, total_weight);

        if total_weight > 0.0 {
            let margin_bps = to_bps(winner.total_weight - runner_up_weight, total_weight);
            if winner.count > 0 && (winner.total_weight - runner_up_weight).abs() < f32::EPSILON {
                self.ambiguity_score_bps = self.ambiguity_score_bps.max(self.convergence_score_bps);
            } else if margin_bps < WEAK_MARGIN_BPS {
                self.ambiguity_score_bps = self
                    .ambiguity_score_bps
                    .max(SCORE_SCALE.saturating_sub(margin_bps));
                self.convergence_score_bps = self
                    .convergence_score_bps
                    .min(SCORE_SCALE.saturating_sub(WEAK_MARGIN_BPS));
            }
        }
    }
}

#[derive(Debug, Default)]
struct InlineFoldState {
    positions: BTreeMap<Position, PositionAccumulator>,
    convergence_sum_bps: u64,
    ambiguity_sum_bps: u64,
}

impl InlineFoldState {
    fn ingest(&mut self, item: FoldItem) {
        let entry = self.positions.entry(item.position).or_default();
        self.convergence_sum_bps = self
            .convergence_sum_bps
            .saturating_sub(entry.convergence_score_bps as u64);
        self.ambiguity_sum_bps = self
            .ambiguity_sum_bps
            .saturating_sub(entry.ambiguity_score_bps as u64);

        entry.add_vote(item.text, item.weight);

        self.convergence_sum_bps = self
            .convergence_sum_bps
            .saturating_add(entry.convergence_score_bps as u64);
        self.ambiguity_sum_bps = self
            .ambiguity_sum_bps
            .saturating_add(entry.ambiguity_score_bps as u64);
    }

    fn metrics(&self) -> (u32, u32) {
        if self.positions.is_empty() {
            return (0, SCORE_SCALE);
        }

        let positions = self.positions.len() as u64;
        (
            (self.convergence_sum_bps / positions) as u32,
            (self.ambiguity_sum_bps / positions) as u32,
        )
    }
}

/// Deterministic reducer.
///
/// Contract:
///   Reducer: Vec<OCRDocument> -> AggregateState
pub fn reduce_documents(docs: Vec<OCRDocument>) -> anyhow::Result<AggregateState> {
    if docs.is_empty() {
        return Err(anyhow::anyhow!("no OCR documents to reduce"));
    }

    let iterations = docs.len() as u32;
    let source = docs
        .iter()
        .map(|d| d.source.as_str())
        .min()
        .unwrap_or("")
        .to_string();
    let document_id = if source.is_empty() {
        Uuid::nil()
    } else {
        Uuid::new_v5(&Uuid::NAMESPACE_URL, source.as_bytes())
    };

    let mut items: Vec<FoldItem> = Vec::new();

    for doc in &docs {
        for page in &doc.pages {
            for (line_idx, line) in page.lines.iter().enumerate() {
                let text = normalize_for_vote(&line.text);
                if text.is_empty() {
                    continue;
                }

                items.push(FoldItem {
                    position: (page.page_number as u32, (line_idx + 1) as u32),
                    text,
                    weight: sanitize_confidence(line.confidence),
                    source: doc.source.clone(),
                });
            }
        }
    }

    if items.is_empty() {
        return Err(anyhow::anyhow!("no non-empty OCR lines to reduce"));
    }

    items.sort_by(|a, b| {
        a.position
            .cmp(&b.position)
            .then_with(|| a.text.cmp(&b.text))
            .then_with(|| b.weight.total_cmp(&a.weight))
            .then_with(|| a.source.cmp(&b.source))
    });

    let mut fold_state = InlineFoldState::default();
    for item in items {
        fold_state.ingest(item);
    }

    let pages = build_pages(&fold_state.positions);
    let fields = build_fields(&pages);
    let cluster_groups = build_cluster_groups(&fold_state.positions);

    let (convergence_score_bps, ambiguity_score_bps) = fold_state.metrics();

    Ok(AggregateState {
        document_id,
        source,
        fields,
        pages,
        convergence_score_bps,
        iterations,
        ambiguity_score_bps,
        cluster_groups,
    })
}
fn prefer_text<'a>(left: &'a str, right: &'a str) -> &'a str {
    if right.len() > left.len() {
        return right;
    }
    if left.len() > right.len() {
        return left;
    }
    if right < left {
        right
    } else {
        left
    }
}

fn build_pages(results: &BTreeMap<Position, PositionAccumulator>) -> Vec<OCRPage> {
    let mut by_page: BTreeMap<u32, Vec<(u32, OCRLine)>> = BTreeMap::new();

    for ((page, line), result) in results {
        by_page.entry(*page).or_default().push((
            *line,
            OCRLine {
                text: result.winner_text.clone(),
                confidence: Some((result.convergence_score_bps as f32) / 10_000.0),
            },
        ));
    }

    by_page
        .into_iter()
        .map(|(page_number, mut lines)| {
            lines.sort_by_key(|(line, _)| *line);
            OCRPage {
                page_number: page_number as usize,
                lines: lines.into_iter().map(|(_, line)| line).collect(),
            }
        })
        .collect()
}

fn build_cluster_groups(
    results: &BTreeMap<Position, PositionAccumulator>,
) -> BTreeMap<usize, BTreeMap<usize, Vec<Vec<String>>>> {
    let mut cluster_groups: BTreeMap<usize, BTreeMap<usize, Vec<Vec<String>>>> = BTreeMap::new();

    for ((page, line), result) in results {
        let alternatives = result.alternatives();
        cluster_groups
            .entry(*page as usize)
            .or_default()
            .insert(
                (*line - 1) as usize,
                alternatives
                    .into_iter()
                    .map(|alt| vec![alt.text; alt.count as usize])
                    .collect(),
            );
    }

    cluster_groups
}

fn build_fields(pages: &[OCRPage]) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    for page in pages {
        for (line_idx, line) in page.lines.iter().enumerate() {
            let key = format!("page_{}_line_{}", page.page_number, line_idx + 1);
            fields.insert(key, line.text.clone());
        }
    }
    fields
}

fn sanitize_confidence(confidence: Option<f32>) -> f32 {
    match confidence {
        Some(value) if value.is_finite() => value.clamp(0.0, 1.0),
        Some(_) => 0.0,
        None => 1.0,
    }
}

fn to_bps(part: f32, total: f32) -> u32 {
    if total <= 0.0 {
        return 0;
    }

    ((part / total) * 10_000.0).round().clamp(0.0, 10_000.0) as u32
}

fn normalize_for_vote(raw: &str) -> String {
    let nfc: String = raw.nfc().collect();
    let trimmed = nfc.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let collapsed = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    harmonize_decimal_comma(&collapsed)
}

fn harmonize_decimal_comma(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(chars.len());

    for (i, ch) in chars.iter().enumerate() {
        if *ch == ',' {
            let prev_digit = i > 0 && chars[i - 1].is_ascii_digit();
            let next_digit = i + 1 < chars.len() && chars[i + 1].is_ascii_digit();
            if prev_digit && next_digit {
                out.push('.');
                continue;
            }
        }
        out.push(*ch);
    }

    out
}

#[allow(dead_code)]
pub fn snapshot_documents(
    docs: Vec<OCRDocument>,
    metadata: SnapshotMetadata,
) -> anyhow::Result<ReducerSnapshot> {
    let state = reduce_documents(docs)?;
    Ok(state.snapshot_with_metadata(metadata))
}

pub fn emit_observation(
    state: &AggregateState,
    observation_id: Uuid,
    created_at: DateTime<Utc>,
    document_type: &str,
    profile: &IngestionProfile,
) -> anyhow::Result<Option<OcrObservation>> {
    let status = state.compute_convergence();
    if status == ObservationStatus::Converged {
        return Ok(None);
    }

    let mut observation = OcrObservation::new(
        observation_id,
        Uuid::new_v5(&Uuid::NAMESPACE_URL, state.source.as_bytes()),
        created_at,
        "reducer.document",
        status,
    );

    observation.value = Some(state.source.clone());
    observation.confidence = Some(state.global_confidence());
    observation.iterations = state.iterations;
    observation.severity = map_severity(
        status,
        document_type,
        &state.source,
        observation.confidence,
        profile,
    );

    match status {
        ObservationStatus::Ambiguous => {
            observation.reason_code = Some("ambiguity_high".to_string());
            observation.note = Some(format!(
                "ambiguity_score_bps={} convergence_score_bps={}",
                state.ambiguity_score_bps,
                state.convergence_score_bps
            ));
        }
        ObservationStatus::Failed => {
            observation.reason_code = Some("reducer_failed".to_string());
            observation.note = Some(format!(
                "pages={} iterations={}",
                state.pages.len(),
                state.iterations
            ));
        }
        ObservationStatus::Converged => {}
    }

    observation.validate()?;
    Ok(Some(observation))
}

#[cfg(test)]
mod tests {
    use super::{reduce_documents, FoldItem, InlineFoldState, PositionAccumulator};
    use crate::ocrys::types::{OCRDocument, OCRLine, OCRPage};

    fn doc(source: &str, text: &str, confidence: f32) -> OCRDocument {
        OCRDocument {
            source: source.to_string(),
            pages: vec![OCRPage {
                page_number: 1,
                lines: vec![OCRLine {
                    text: text.to_string(),
                    confidence: Some(confidence),
                }],
            }],
        }
    }

    #[test]
    fn majority_vote_wins() {
        let reduced = reduce_documents(vec![
            doc("a", "45,20", 0.9),
            doc("b", "45,20", 0.8),
            doc("c", "45.20", 0.6),
        ])
        .expect("reduce");

        assert_eq!(reduced.pages[0].lines[0].text, "45.20");
    }

    #[test]
    fn higher_confidence_minority_can_win() {
        let reduced = reduce_documents(vec![
            doc("a", "ABC", 0.95),
            doc("b", "ABD", 0.40),
            doc("c", "ABD", 0.30),
        ])
        .expect("reduce");

        assert_eq!(reduced.pages[0].lines[0].text, "ABC");
    }

    #[test]
    fn input_order_independent_output() {
        let a = doc("a", "line one", 0.91);
        let b = doc("b", "line one", 0.90);
        let c = doc("c", "line two", 0.95);

        let state_1 = reduce_documents(vec![a.clone(), b.clone(), c.clone()]).expect("reduce");
        let state_2 = reduce_documents(vec![c, a, b]).expect("reduce");

        let left = serde_json::to_string(&state_1).expect("serialize left");
        let right = serde_json::to_string(&state_2).expect("serialize right");
        assert_eq!(left, right);
    }

    #[test]
    fn duplicate_doc_no_phantom_clusters() {
        let base = doc("a", "line one", 0.90);

        let once = reduce_documents(vec![base.clone()]).expect("reduce once");
        let twice = reduce_documents(vec![base.clone(), base]).expect("reduce twice");

        assert_eq!(once.cluster_groups.len(), twice.cluster_groups.len());
        assert_eq!(once.pages[0].lines[0].text, twice.pages[0].lines[0].text);
    }

    #[test]
    fn scores_are_bounded() {
        let reduced = reduce_documents(vec![
            doc("a", "alpha", 0.10),
            doc("b", "beta", 0.30),
            doc("c", "gamma", 0.60),
        ])
        .expect("reduce");

        assert!(reduced.convergence_score_bps <= 10_000);
        assert!(reduced.ambiguity_score_bps <= 10_000);
    }

    #[test]
    fn tie_increases_ambiguity() {
        let reduced = reduce_documents(vec![
            doc("a", "A", 0.50),
            doc("b", "B", 0.50),
        ])
        .expect("reduce");

        assert!(reduced.ambiguity_score_bps >= 5_000);
    }

    #[test]
    fn leader_changes_when_stronger_candidate_arrives() {
        let mut pos = PositionAccumulator::default();

        pos.add_vote("ALPHA".to_string(), 0.45);
        assert_eq!(pos.winner_text, "ALPHA");

        pos.add_vote("BETA".to_string(), 0.95);
        assert_eq!(pos.winner_text, "BETA");
    }

    #[test]
    fn convergence_increases_with_confirming_evidence() {
        let mut state = InlineFoldState::default();

        state.ingest(FoldItem {
            position: (1, 1),
            text: "ALPHA".to_string(),
            weight: 0.60,
            source: "a".to_string(),
        });
        state.ingest(FoldItem {
            position: (1, 1),
            text: "BETA".to_string(),
            weight: 0.55,
            source: "b".to_string(),
        });
        let (before, _) = state.metrics();

        state.ingest(FoldItem {
            position: (1, 1),
            text: "ALPHA".to_string(),
            weight: 0.90,
            source: "c".to_string(),
        });
        let (after, _) = state.metrics();

        assert!(after >= before, "confirming evidence should not decrease convergence");
    }

    #[test]
    fn ambiguity_increases_on_tie() {
        let mut state = InlineFoldState::default();

        state.ingest(FoldItem {
            position: (1, 1),
            text: "A".to_string(),
            weight: 0.90,
            source: "a".to_string(),
        });
        let (_, ambiguity_before) = state.metrics();

        state.ingest(FoldItem {
            position: (1, 1),
            text: "B".to_string(),
            weight: 0.90,
            source: "b".to_string(),
        });
        let (_, ambiguity_after) = state.metrics();

        assert!(
            ambiguity_after > ambiguity_before,
            "equal competing candidates should increase ambiguity"
        );
    }
}
