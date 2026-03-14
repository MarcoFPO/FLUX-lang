#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/release.sh <version>
# Example: ./scripts/release.sh 1.0.0
# Creates and pushes a git tag v<version>, triggering the release workflow.

VERSION="${1:-}"

if [ -z "$VERSION" ]; then
    echo "Usage: $0 <version>"
    echo "Example: $0 1.0.0"
    exit 1
fi

TAG="v${VERSION}"

if git rev-parse "$TAG" >/dev/null 2>&1; then
    echo "Error: Tag $TAG already exists"
    exit 1
fi

# Ensure we're on main and up to date
BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [ "$BRANCH" != "main" ]; then
    echo "Warning: Not on main branch (currently on $BRANCH)"
    read -rp "Continue anyway? [y/N] " confirm
    [ "$confirm" = "y" ] || exit 1
fi

echo "Creating tag $TAG..."
git tag -a "$TAG" -m "Release $TAG"

echo "Pushing tag $TAG..."
git push origin "$TAG"

echo "Done. Release workflow will run at:"
echo "  https://github.com/$(git remote get-url origin | sed 's|.*github.com[:/]||;s|\.git$||')/actions"
