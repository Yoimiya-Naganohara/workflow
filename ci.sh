#!/usr/bin/env bash
set -euo pipefail

# CI gates for workflow project.
# Usage: ./ci.sh [--fix]

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

pass() { echo -e "  ${GREEN}✅${NC} $1"; }
fail() { echo -e "  ${RED}❌${NC} $1"; }

echo "=== CI Gates ==="

# Gate 1: cargo check
echo -e "\n1. cargo check"
if cargo check 2>&1; then pass "check clean"; else fail "check failed"; exit 1; fi

# Gate 2: format
echo -e "\n2. cargo fmt"
if cargo fmt --check 2>&1; then
    pass "format clean"
else
    if [[ "${1:-}" == "--fix" ]]; then
        cargo fmt
        pass "format fixed"
    else
        fail "format issues (run ./ci.sh --fix to auto-fix)"
        exit 1
    fi
fi

# Gate 3: clippy
echo -e "\n3. cargo clippy"
if cargo clippy -- -D warnings 2>&1; then
    pass "clippy clean"
else
    fail "clippy warnings"
    exit 1
fi

# Gate 4: tests
echo -e "\n4. cargo test"
if cargo test 2>&1; then
    pass "all tests passed"
else
    fail "tests failed"
    exit 1
fi

# Gate 5: docs
echo -e "\n5. cargo doc"
if cargo doc --no-deps 2>&1; then
    pass "docs built"
else
    fail "docs failed"
    exit 1
fi

echo -e "\n${GREEN}=== All CI gates passed ===${NC}"
