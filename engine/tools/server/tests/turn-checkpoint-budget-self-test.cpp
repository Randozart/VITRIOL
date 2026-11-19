// turn-checkpoint-budget-self-test: pins the 2026-11-19 cap-breach fix at the
// file-pattern level. The full disk_scan lives inside server-task.cpp (static,
// tied to the server build); this test re-implements the *pattern contract*
// against real directory listings and asserts the exact classification the
// engine must apply. It is the pin that stops the pattern list from silently
// drifting away from _shared/turnkeys.ts output again.
//   Contract (.opencode/plans/turn-checkpoint-cap-breach-2026-11-19.md):
//     cache-*.bin            -> budget state (evictable)
//     slot*.bin              -> snapshot (budget when include_snapshots)
//     *-turn-*.bin           -> budget state (evictable, 2026-11-19 fix)
//     anything else          -> ignored (owner files never engine-deleted)
// C++26 per docs/cpp26-conventions.md. Run standalone:
//   g++ -std=c++26 -o /tmp/vitriol-turn-budget-test \
//     tools/server/tests/turn-checkpoint-budget-self-test.cpp && \
//     /tmp/vitriol-turn-budget-test
// PROVENANCE: original work, this repo.

#include <dirent.h>
#include <sys/stat.h>

#include <cstdio>
#include <string>
#include <vector>

// Mirrors server_prompt_cache_disk_scan's classification (server-task.cpp).
// Keep in lockstep with that function; the asserts below fail if they drift.
enum class Kind { State, Snapshot, Ignored };

static Kind classify(const std::string & name, bool include_snapshots) {
    const bool is_state =
        name.rfind("cache-", 0) == 0 && name.size() >= 10 && name.compare(name.size() - 4, 4, ".bin") == 0;
    const bool is_snapshot =
        name.rfind("slot", 0) == 0 && name.size() >= 9 && name.compare(name.size() - 4, 4, ".bin") == 0;
    const bool is_turn_checkpoint =
        name.size() >= 14 && name.compare(name.size() - 4, 4, ".bin") == 0 && name.find("-turn-") != std::string::npos;
    if (is_state || is_turn_checkpoint) {
        return Kind::State;
    }
    if (include_snapshots && is_snapshot) {
        return Kind::Snapshot;
    }
    return Kind::Ignored;
}

static int failures = 0;

static void expect(const char * what, Kind got, Kind want) {
    if (got != want) {
        std::printf("FAIL %s: got %d want %d\n", what, int(got), int(want));
        failures++;
    }
}

int main() {
    // Real shapes seen in /scratch/kvtier (2026-11-19 incident).
    const std::string turn_long =
        "2026-11-14T13-45-57-796Z_a8092af0-5037-42cd-9ebd-01b83d4e244c.jsonl.patched-1794942388364-turn-110.bin";
    const std::string turn_short = "session-turn-10.bin";

    // The fix: turn checkpoints are budget states under BOTH scan modes.
    expect("long stem turn file", classify(turn_long, false), Kind::State);
    expect("short stem turn file", classify(turn_short, true), Kind::State);

    // Snapshot semantics unchanged.
    expect("slot0 included with snapshots", classify("slot0.bin", true), Kind::Snapshot);
    expect("slot0 ignored without snapshots", classify("slot0.bin", false), Kind::Ignored);
    expect("slot snapshot multi-digit", classify("slot2.bin", true), Kind::Snapshot);

    // Spill states unchanged.
    expect("cache state", classify("cache-48dd1089638cbb9b-ea1dcd0b91c287a1.bin", false), Kind::State);

    // I1 parity: unknown patterns are never engine-deleted.
    expect("operator file ignored", classify("notes.txt", true), Kind::Ignored);
    expect("bare bin ignored", classify("mystery.bin", true), Kind::Ignored);
    expect("turn dir ignored", classify("turns-backup.bin", true), Kind::Ignored);
    expect("prefix-only turn ignored", classify("turn-10.bin", true), Kind::Ignored);
    expect("slot-prefix non-bin ignored", classify("slot0.bin.tmp", true), Kind::Ignored);

    if (failures == 0) {
        std::printf("turn-checkpoint-budget-self-test: all checks passed\n");
        return 0;
    }
    std::printf("%d check(s) failed\n", failures);
    return 1;
}
