# Prepares a Mac to build Voice by Superteem for the Mac and the iPhone. Source it: source desktop/build-env.sh
# The Mac's own tools do most of it: Xcode's clang and libclang, and CMake from Homebrew or GitHub's runners.
# This makes the local copy of whisper-rs-sys that Cargo.toml's [patch] points at, as build-env.ps1 does on
# Windows, and sets how whisper.cpp is compiled.
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
vendor="$here/vendor/whisper-rs-sys"

command -v cmake >/dev/null || { echo "CMake is required: brew install cmake" >&2; return 1; }

if [ ! -f "$vendor/build.rs" ]; then
  registry=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/whisper-rs-sys-0.15.0 2>/dev/null | head -n 1 || true)
  if [ -z "$registry" ]; then
    # The app's own manifest can't fetch it while the patch path is missing, so a scratch crate does.
    scratch="${TMPDIR:-/tmp}/stv-fetch-whisper"
    mkdir -p "$scratch/src"
    : > "$scratch/src/lib.rs"
    printf '[package]\nname = "fetch"\nversion = "0.0.0"\nedition = "2021"\n[dependencies]\nwhisper-rs-sys = "=0.15.0"\n' > "$scratch/Cargo.toml"
    cargo fetch --manifest-path "$scratch/Cargo.toml" || return 1
    registry=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/whisper-rs-sys-0.15.0 | head -n 1)
  fi
  mkdir -p "$(dirname "$vendor")"
  cp -R "$registry" "$vendor" || return 1
fi

# The build script passes GGML_* variables to whisper.cpp's CMake but doesn't rebuild when they change, so
# a copy built for the wrong processor would be kept. This makes cargo rebuild it when they change.
python3 - "$vendor/build.rs" <<'PY'
import sys
path = sys.argv[1]
text = open(path).read()
if "rerun-if-env-changed=GGML_NATIVE" not in text:
    marker = "    // Allow passing any WHISPER or CMAKE compile flags"
    if marker not in text:
        sys.exit("The whisper-rs-sys build script changed; update build-env.sh")
    watch = "".join(f'    println!("cargo:rerun-if-env-changed={v}");\n'
                    for v in ("GGML_NATIVE", "GGML_AVX", "GGML_AVX2", "GGML_FMA", "GGML_F16C", "GGML_AVX512"))
    open(path, "w").write(text.replace(marker, watch + marker))
PY
[ $? -eq 0 ] || return 1

# whisper.cpp is otherwise built for the build machine's own processor, and a newer chip on GitHub's
# runners would make the app crash on older Macs, as AVX-512 once did on Windows. Without it, Xcode's
# clang builds for the oldest Apple silicon Mac (and the oldest iPhone the app supports), and whisper.cpp
# uses Apple's Accelerate framework for the heavy maths either way.
export GGML_NATIVE=OFF
