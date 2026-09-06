#!/usr/bin/env python3
"""E34 workload driver: drives opencode-style coding traffic against a
llama-server to profile expert usage. Usage:
    vitriol-profile-workload.py A|B [host:port]
Task A = code comprehension/debug/refactor (paste-heavy)
Task B = write-new-code (generation-heavy)
"""
import json, sys, time, urllib.request

HOST = sys.argv[2] if len(sys.argv) > 2 else "127.0.0.1:8080"
URL = f"http://{HOST}/v1/chat/completions"

# real snippets from the repo so the traffic is genuinely code-shaped
SNIPPET_TOUCH = """
static void vitriol_sycl_touch_pages(const void * ptr, size_t size) {
    static const size_t page = (size_t) sysconf(_SC_PAGESIZE);
    if (!ptr || size == 0 || page == 0) { return; }
    uintptr_t base = (uintptr_t) ptr & ~(page - 1);
    madvise((void *) base, size + (uintptr_t)ptr - base + page, MADV_WILLNEED);
    const volatile char * p = (const volatile char *) ptr;
    for (size_t off = 0; off < size; off += page) { (void) p[off]; }
    (void) p[size - 1];
}
"""
SNIPPET_EVICT = """
    int slot;
    {
        std::lock_guard<std::mutex> lock(g_lru_mtx);
        if ((int)g_lru_map.size() < g_lru_num_slots) {
            slot = (int)g_lru_map.size();
        } else {
            LRUKey evict = g_lru_order.back();
            for (auto it = std::prev(g_lru_order.end()); it != g_lru_order.begin(); --it) {
                auto mit = g_lru_map.find(*it);
                int s = (mit != g_lru_map.end()) ? mit->second : -1;
                if (s >= 0 && s < g_lru_num_slots) {
                    auto status = g_lru_slot_events[s].get_info<sycl::info::event::command_execution_status>();
                    if (status == sycl::info::event_command_status::complete) { evict = *it; break; }
                }
            }
            g_lru_order.remove(evict);
            auto eit = g_lru_map.find(evict);
            slot = (eit != g_lru_map.end()) ? eit->second : 0;
            if (eit != g_lru_map.end()) g_lru_map.erase(eit);
            g_lru_stats.evictions++;
        }
        g_lru_map[key] = slot;
        g_lru_order.push_front(key);
    }
"""

TASKS = {
"A": [
    ("Explain what this function does, then list any bugs you see:\n```cpp" + SNIPPET_TOUCH + "```", 140),
    ("Review this eviction loop for race conditions and suggest a fix:\n```cpp" + SNIPPET_EVICT + "```", 140),
    ("Refactor this C++ to reduce lock contention. Show only the changed function:\n```cpp" + SNIPPET_EVICT + "```", 130),
    ("Here is a CMake snippet. What does GGML_SYCL get from it and why does oneDNN matter?\n```cmake\nfind_package(DNNL QUIET)\nif (DNNL_FOUND)\n  add_compile_definitions(GGML_SYCL_DNNL=1)\n  target_link_libraries(ggml-sycl PRIVATE DNNL::dnnl)\nendif()\n```", 90),
    ("Why can a Level-Zero copy engine crash when reading mmap'd file-backed pages on an iGPU? Answer in <=5 sentences.", 110),
    ("Find the bug: 'for (size_t off = 0; off < size; off += page) { (void)p[off]; }' intended to touch every page of a range. Does it? Why/why not?", 90),
    ("Summarize this diff in 3 bullets:\n```diff\n-    if (expert_size > g_lru_slot_size)\n-        return nullptr;\n+    if (expert_size > g_lru_slot_size && !lru_resize(expert_size))\n+        return nullptr;\n```", 80),
    ("Given this madvise+touch pattern, estimate the syscall count per 1MB slice with 4KB pages, and say whether MADV_WILLNEED reduces page faults or just warms the cache.", 110),
],
"B": [
    ("Write a Python function that parses CSV lines with quoted fields and escaped quotes, without using the csv module. Include a docstring.", 150),
    ("Write a bash script that finds the 10 largest files under /var/log and prints their sizes human-readable.", 120),
    ("Implement binary search on a rotated sorted array in C++. Handle duplicates.", 130),
    ("Write a Python asyncio worker that fetches URLs with a token-bucket rate limit of 5 req/s.", 140),
    ("Write a SQL query using a window function: top 3 customers by revenue per month, with month-over-month delta.", 120),
    ("Implement an LRU cache class in Python with O(1) get/put using an OrderedDict-free approach.", 130),
    ("Write a regex validating ISO-8601 timestamps, then explain each component briefly.", 110),
    ("Draft a CMake snippet that builds a SYCL target with the Intel oneAPI compiler and links MKL, with comments.", 120),
]}

def ask(prompt, max_tokens):
    body = json.dumps({"model": "qwen", "messages": [{"role": "user", "content": prompt}],
                       "max_tokens": max_tokens, "temperature": 0.7}).encode()
    req = urllib.request.Request(URL, data=body, headers={"Content-Type": "application/json"})
    t0 = time.time()
    with urllib.request.urlopen(req, timeout=900) as r:
        d = json.loads(r.read())
    t = d.get("timings", {})
    print(f"  [{time.strftime('%H:%M:%S')}] {t.get('prompt_n','?')} in / {t.get('predicted_n','?')} out "
          f"pp={t.get('prompt_per_second',0):.1f} tg={t.get('predicted_per_second',0):.2f} t/s "
          f"({time.time()-t0:.0f}s)", flush=True)

def main():
    task = sys.argv[1]
    prompts = TASKS[task]
    print(f"== task {task}: {len(prompts)} prompts ==", flush=True)
    for i, (p, mt) in enumerate(prompts):
        for attempt in range(3):
            try:
                ask(p, mt)
                break
            except Exception as e:
                print(f"  retry {attempt+1} after error: {e}", flush=True)
                time.sleep(10)
    print(f"== task {task} done ==", flush=True)

main()
