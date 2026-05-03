#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
test_root="$repo_root/tests"
scope="${1:-all}"

mkdir -p "$test_root"

if [[ "$scope" == "--clear-all" ]]; then
    cleared=0

    while IFS= read -r config_path; do
        rm "$config_path"
        echo "removed ${config_path#$repo_root/}"
        cleared=$((cleared + 1))

        config_dir="$(dirname "$config_path")"
        rmdir "$config_dir" 2>/dev/null || true
    done < <(
        find "$test_root" \
            -path '*/target' -prune -o \
            -path '*/opt' -prune -o \
            -path '*/.cargo/config.toml' -type f -print | sort
    )

    echo "removed $cleared test Cargo config file(s)"
    exit 0
fi

move_into_test() {
    local name="$1"
    local source="$repo_root/$name"
    local dest="$test_root/$name"

    if [[ ! -e "$source" ]]; then
        return
    fi

    if [[ -e "$dest" ]]; then
        echo "skip move: tests/$name already exists"
        return
    fi

    mv "$source" "$dest"
    echo "moved $name -> tests/$name"
}

# Existing generated/test project directories live at the repo root in older
# checkouts. Keep this list narrow so source/tooling directories stay put.
move_into_test "Rec_Get_Test"
move_into_test "Copy_Struct_Test"
move_into_test "Copy_Struct2_Test"
move_into_test "Rust_Out"

configured=0
manifest_list() {
    if [[ "$scope" == "all" ]]; then
        find "$test_root" \
            -path '*/target' -prune -o \
            -path '*/opt' -prune -o \
            -name Cargo.toml -type f -print | sort
        return
    fi

    local project_path="$scope"
    if [[ "$project_path" != /* ]]; then
        project_path="$repo_root/$project_path"
    fi

    if [[ ! -e "$project_path" ]]; then
        echo "project path does not exist: $scope" >&2
        return 1
    fi

    if [[ -f "$project_path" ]]; then
        if [[ "$(basename "$project_path")" != "Cargo.toml" ]]; then
            echo "project file is not Cargo.toml: $scope" >&2
            return 1
        fi
        printf '%s\n' "$project_path"
        return
    fi

    if [[ -f "$project_path/Cargo.toml" ]]; then
        printf '%s\n' "$project_path/Cargo.toml"
        return
    fi

    find "$project_path" \
        -path '*/target' -prune -o \
        -path '*/opt' -prune -o \
        -name Cargo.toml -type f -print | sort
}

while IFS= read -r manifest; do
    package_root="$(dirname "$manifest")"
    config_dir="$package_root/.cargo"
    config_path="$config_dir/config.toml"
    root_manifest="$(realpath --relative-to="$package_root" "$repo_root/Cargo.toml")"

    mkdir -p "$config_dir"
    cat > "$config_path" <<EOF_CONFIG
[alias]
opt = "run --manifest-path $root_manifest --bin cargo-opt --"
run-opt = "run --manifest-path opt/Cargo.toml"
clean-opt = "clean --manifest-path opt/Cargo.toml"
EOF_CONFIG

    echo "configured ${config_path#$repo_root/}"
    configured=$((configured + 1))
done < <(manifest_list)

echo "configured $configured test Cargo package(s)"
