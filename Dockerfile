# Build stage
FROM rust:latest as builder

WORKDIR /app

# Install build dependencies required for tesseract bindings and native crates
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    clang \
    build-essential \
    libtesseract-dev \
    libleptonica-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy project
COPY Cargo.toml Cargo.lock* ./
COPY src ./src

# Build release binary
RUN cargo build --release

# Runtime stage
FROM rust:latest

# Install tesseract, language packs and runtime libs on the same distro base
RUN apt-get update && apt-get install -y --no-install-recommends \
    tesseract-ocr \
    tesseract-ocr-all \
    libtesseract5 \
    libgomp1 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/optimo /usr/local/bin/optimo

# Create data directory for outputs
RUN mkdir -p /app/data

ENTRYPOINT ["optimo"]
CMD ["--help"]
