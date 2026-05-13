#!/bin/bash
# VITRIOL Alka Mock Runner
# Runs Alka recipes through the mock executor (no kernel/hardware needed)
#
# Usage:
#   ./run_mock.sh                  # Run all recipes
#   ./run_mock.sh benchmark_35b    # Run specific recipe
#   ./run_mock.sh --compile-only   # Just compile, don't execute

set -e

ALKA_BIN="/home/randozart/Desktop/Projects/alka-lang/zig-out/bin/alka"
ALKA_DIR="/home/randozart/Desktop/Projects/VITRIOL/alka"
RECIPES_DIR="$ALKA_DIR/recipes"
VIAL="$ALKA_DIR/vials/vitriol_rig.alkavl"
RESULTS_DIR="$ALKA_DIR/results"

mkdir -p "$RESULTS_DIR"

if [ ! -f "$ALKA_BIN" ]; then
    echo "Error: Alka compiler not found at $ALKA_BIN"
    echo "Build it first: cd /home/randozart/Desktop/Projects/alka-lang && zig build"
    exit 1
fi

if [ ! -f "$VIAL" ]; then
    echo "Error: Vial not found at $VIAL"
    exit 1
fi

echo "=== VITRIOL Alka Mock Runner ==="
echo "Vial: $VIAL"
echo ""

run_recipe() {
    local name=$1
    local recipe="$RECIPES_DIR/${name}.alka"

    if [ ! -f "$recipe" ]; then
        echo "[SKIP] $name: recipe not found at $recipe"
        return
    fi

    echo "--- $name ---"
    echo "Compiling: ${name}.alka"

    # Compile
    local compile_output
    compile_output=$($ALKA_BIN "$recipe" "$VIAL" 2>&1) || true
    echo "$compile_output"

    local alkas="${recipe}.alkas"
    local azoth="${recipe}.azoth"

    if [ ! -f "$alkas" ]; then
        echo "  [WARN] No .alkas produced — recipe may use unsupported instructions"
        echo ""
        return
    fi

    echo ""
    echo "Mock executing: ${name}.alkas"
    local mock_output
    mock_output=$($ALKA_BIN --mock "$alkas" 2>&1) || true
    echo "$mock_output"

    # Save outputs
    echo "$compile_output" > "$RESULTS_DIR/${name}_compile.txt"
    echo "$mock_output" > "$RESULTS_DIR/${name}_mock.txt"

    echo ""
    echo "Results saved to $RESULTS_DIR/${name}_*.txt"
    echo ""
}

if [ "$1" = "--compile-only" ]; then
    # Compile all recipes without executing
    for recipe in "$RECIPES_DIR"/*.alka; do
        local name=$(basename "$recipe" .alka)
        echo "--- $name (compile only) ---"
        $ALKA_BIN "$recipe" "$VIAL" 2>&1 || true
        echo ""
    done
elif [ -n "$1" ]; then
    # Run specific recipe
    run_recipe "$1"
else
    # Run all recipes
    for recipe in "$RECIPES_DIR"/*.alka; do
        local name=$(basename "$recipe" .alka)
        run_recipe "$name"
    done
fi

echo "=== Mock Run Complete ==="
