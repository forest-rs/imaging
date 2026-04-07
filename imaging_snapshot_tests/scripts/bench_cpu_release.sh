#!/usr/bin/env bash
set -euo pipefail

backend="${1:-tiny_skia}"
loops="${2:-}"

case_args=()
if [[ $# -ge 3 ]]; then
  case_args=("${@:3}")
fi

if [[ ${#case_args[@]} -eq 0 ]]; then
  case_args=(
    gm_image_brushes
    gm_gradients_sweep
    gm_gradients_two_point_radial
    gm_mask_alpha
    gm_mask_luminance
    gm_blend_grid
    gm_group_blur_filter
    gm_blurred_rounded_rect_variants
    gm_svg_layered_card
  )
fi

case "${backend}" in
  tiny_skia)
    feature="tiny_skia"
    target_dir="/tmp/imaging-profile-tiny"
    default_loops=80
    ;;
  skia)
    feature="skia"
    target_dir="/tmp/imaging-profile-skia-release"
    default_loops=80
    ;;
  vello_cpu)
    feature="vello_cpu"
    target_dir="/tmp/imaging-profile-vello-cpu"
    default_loops=80
    ;;
  *)
    echo "unsupported backend: ${backend}" >&2
    echo "expected one of: tiny_skia, skia, vello_cpu" >&2
    exit 2
    ;;
esac

if [[ -z "${loops}" ]]; then
  loops="${default_loops}"
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

echo "backend=${backend} loops=${loops}"
for case_name in "${case_args[@]}"; do
  IMAGING_PROFILE_BACKEND="${backend}" \
  IMAGING_CASE="${case_name}" \
  IMAGING_PROFILE_LOOPS="${loops}" \
  CARGO_TARGET_DIR="${target_dir}" \
    cargo run --release -p imaging_snapshot_tests --features "${feature}" --bin profile_render
done
