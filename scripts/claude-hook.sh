#!/usr/bin/env bash
#
# Claude Code Task Completion Hook
#
# This hook runs when Claude believes a task is complete.
# It validates that all tests pass before allowing task completion.
#
# Exit codes:
#   0 - Tests pass, task can be marked complete
#   2 - Tests fail, Claude should fix failures before completing

set -euo pipefail

# Change to project root (script is in /scripts subdirectory)
cd "$(dirname "$0")/.."

# Create temporary file for test output
TEST_OUTPUT=$(mktemp)
trap 'rm -f "$TEST_OUTPUT"' EXIT

# Run tests silently, capturing output
if cargo test > "$TEST_OUTPUT" 2>&1; then
    # Tests passed - remind Claude to update roadmap
    echo "" >&2
    echo "✓ All tests passed. Task validation successful." >&2
    echo "" >&2
    echo "Next steps:" >&2
    echo "1. Update ROADMAP.md to mark completed tasks with ✅" >&2
    echo "2. Add implementation notes and learnings to the roadmap" >&2
    echo "3. Review ROADMAP.md to identify the next task" >&2
    echo "4. Inform the user of progress and suggest next steps" >&2
    echo "" >&2
    exit 0
else
    # Tests failed - show output and provide guidance to Claude
    cat "$TEST_OUTPUT" >&2
    echo "" >&2
    echo "✗ Tests failed. Cannot mark task complete until all tests pass." >&2
    echo "" >&2
    echo "GUIDANCE FOR CLAUDE:" >&2
    echo "==================" >&2
    echo "" >&2
    echo "Focus on fixing the failing tests shown above before marking this task complete." >&2
    echo "" >&2
    echo "Common approaches:" >&2
    echo "1. Read the test failure output carefully to identify the root cause" >&2
    echo "2. Review the failing test code to understand expected vs actual behavior" >&2
    echo "3. Check if recent changes introduced regressions in existing tests" >&2
    echo "4. Ensure new functionality has corresponding tests that pass" >&2
    echo "5. Run 'cargo test <test_name>' to focus on specific failing tests" >&2
    echo "" >&2
    echo "Remember: The codebase must be in a healthy state (all tests passing)" >&2
    echo "before any task is considered complete." >&2
    echo "" >&2

    exit 2
fi
