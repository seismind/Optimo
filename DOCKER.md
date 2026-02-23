# Docker quick run

Build the image:

```bash
docker build -t optimo:latest .
```

Run the container interactively to inspect files and debug the pipeline:

```bash
docker run -it -v "$(pwd):/workspace" optimo /bin/bash
```

Run the binary against a sample image (mount `data` to persist outputs):

```bash
docker run --rm -v "$(pwd)/data:/app/data" optimo /workspace/sample.png
```

Notes:
- The image installs `tesseract-ocr-all` (large) so initial build may take time.
- Mounting the project directory (`/workspace`) lets you inspect `data/` and run the binary manually inside the container.
## Prepare `data/` on the host (recommended)

It's best to prepare the `data/` directory on the host before running the container so files created by the service are owned by your user and not `root`.

You can use the included helper script to create the directories and fix ownership:

```bash
./scripts/setup_data.sh
# or manually:
mkdir -p data/ocrys/latest
chown -R $(id -u):$(id -g) data
```

If you prefer to run the container as your user instead, start it with `-u $(id -u):$(id -g)`.
