#!/usr/bin/env bash
# Turn any free Linux machine into the channel's render worker (a GitHub self-hosted runner).
#
# Works on: Ubuntu/Debian x86_64, including WSL2 on a Windows PC and an old laptop.
# Possibly works on: ARM64 (e.g. Oracle Cloud Always Free Ampere VM) - only if a bpy wheel
# exists for that architecture; the script checks and stops with a clear message if not.
#
# Usage:
#   1. GitHub repo -> Settings -> Actions -> Runners -> New self-hosted runner -> copy the token
#   2. bash deploy/setup-runner.sh https://github.com/OWNER/REPO <TOKEN>
#
# The runner then picks up `final` workflow runs. Nothing is exposed to the internet:
# the runner only makes outbound connections to GitHub.

set -euo pipefail

REPO_URL="${1:?usage: setup-runner.sh https://github.com/OWNER/REPO <registration-token>}"
TOKEN="${2:?missing registration token (Settings -> Actions -> Runners -> New self-hosted runner)}"
RUNNER_DIR="${RUNNER_DIR:-$HOME/forge-runner}"
VENV="${VENV:-$HOME/.forge-bpy}"
NAME="${RUNNER_NAME:-$(hostname)-render}"

case "$(uname -m)" in
  x86_64) ARCH=x64 ;;
  aarch64|arm64) ARCH=arm64 ;;
  *) echo "unsupported CPU architecture: $(uname -m)"; exit 1 ;;
esac

echo "==> system packages"
sudo apt-get update -qq
sudo apt-get install -y -qq curl git build-essential pkg-config jq ffmpeg fonts-dejavu-core \
  libegl1 libgl1 libxkbcommon0 libsm6 libxi6 libxxf86vm1 libxfixes3 libxrender1

echo "==> Python 3.11 + Blender module (bpy)"
command -v uv >/dev/null || curl -LsSf https://astral.sh/uv/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
uv python install 3.11
uv venv --allow-existing --python 3.11 "$VENV"
if ! VIRTUAL_ENV="$VENV" uv pip install "bpy==4.5.*"; then
  echo "!! No bpy build for $(uname -m). Install Blender 4.5 for this architecture instead and"
  echo "   set FORGE_BLENDER='blender -b --factory-startup --python' in $RUNNER_DIR/.env"
  exit 1
fi
"$VENV/bin/python" -c "import bpy; print('Blender', bpy.app.version_string)"

echo "==> Rust"
command -v cargo >/dev/null || curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
# shellcheck disable=SC1091
source "$HOME/.cargo/env"

echo "==> GitHub Actions runner"
VERSION=$(curl -s https://api.github.com/repos/actions/runner/releases/latest | jq -r .tag_name | sed 's/^v//')
mkdir -p "$RUNNER_DIR"
cd "$RUNNER_DIR"
if [ ! -f ./config.sh ]; then
  curl -sSL -o runner.tar.gz "https://github.com/actions/runner/releases/download/v${VERSION}/actions-runner-linux-${ARCH}-${VERSION}.tar.gz"
  tar xzf runner.tar.gz && rm runner.tar.gz
fi
./config.sh --url "$REPO_URL" --token "$TOKEN" --name "$NAME" --labels render --unattended --replace

# Environment the runner passes to every job.
cat > .env <<EOF
FORGE_BLENDER=$VENV/bin/python
PATH=$HOME/.cargo/bin:$HOME/.local/bin:/usr/local/bin:/usr/bin:/bin
EOF

if pidof systemd >/dev/null 2>&1; then
  sudo ./svc.sh install "$USER"
  sudo ./svc.sh start
  echo "==> runner '$NAME' installed as a service and running"
else
  echo "==> no systemd (WSL without systemd?). Start the runner with:  $RUNNER_DIR/run.sh"
fi

cat <<EOF

Done. Next:
  * GitHub -> Actions -> final -> Run workflow (pick the video and profile)
  * GPU in this machine? set gpu = true in forge.toml (Cycles uses CUDA/OptiX/HIP).
  * Remove later:  cd $RUNNER_DIR && sudo ./svc.sh uninstall && ./config.sh remove --token <TOKEN>
EOF
