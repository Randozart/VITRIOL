"""Aider-style repo map builder for Hermetis (P3.3).

Builds a concise map of a repository: per-file symbol signatures, ranked by
file-dependency references, trimmed to a token budget. The map is stored as
versioned knowledge nodes (per file, keyed by git_rev) so file edits supersede
the old node.

Symbol extraction is a pragmatic regex pass across common languages (tree-sitter
is the documented upgrade path). The dependency graph is import-derived; ranking
uses in-degree (how many files depend on a file), Aider-style.
"""
import os
import re
import subprocess

from .scorer import estimate_tokens

# Directories/files never part of the map.
SKIP_DIRS = {'.git', '.opencode', '__pycache__', 'node_modules', 'target',
             'build', 'dist', '.venv', 'venv', '.vitriol', '.cache'}
SKIP_EXTS = {'.pyc', '.pyo', '.so', '.o', '.dll', '.dylib', '.exe', '.gguf',
             '.bin', '.lock', '.min.js', '.map'}

# Per-language symbol patterns: (regex, kind). Applied per line.
LANG_PATTERNS = {
    'python': [
        (r'^\s*def\s+(\w+)\s*\(([^)]*)\)', 'def'),
        (r'^\s*class\s+(\w+)\s*[(:]', 'class'),
        (r'^\s+def\s+(\w+)\s*\(([^)]*)\)', 'method'),
    ],
    'rust': [
        (r'^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)\s*(?:<[^>]*>)?\s*\(([^)]*)\)', 'fn'),
        (r'^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|trait)\s+(\w+)', 'type'),
        (r'^\s+fn\s+(\w+)\s*\(([^)]*)\)', 'method'),
    ],
    'typescript': [
        (r'^(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+(\w+)\s*\(([^)]*)\)', 'function'),
        (r'^(?:export\s+)?(?:abstract\s+)?class\s+(\w+)', 'class'),
        (r'^(?:export\s+)?(?:interface|type)\s+(\w+)', 'type'),
        (r'^\s{2}(?:async\s+)?(\w+)\s*\(([^)]*)\)\s*[:\{]', 'method'),
    ],
    'go': [
        (r'^func\s+(\w+)\s*\(([^)]*)\)', 'func'),
        (r'^func\s+\([^)]*\)\s+(\w+)\s*\(([^)]*)\)', 'method'),
        (r'^type\s+(\w+)\s+(?:struct|interface)', 'type'),
    ],
    'c_cpp': [
        (r'^(?:static\s+|inline\s+|extern\s+)?[\w:<>*& ]+\s+(\w+)\s*\(([^)]*)\)\s*\{?', 'func'),
        (r'^class\s+(\w+)', 'class'),
        (r'^struct\s+(\w+)', 'struct'),
    ],
}

# Import patterns per language: extract a module token that may map to a file.
IMPORT_PATTERNS = {
    'python': [r'^\s*(?:import|from)\s+([\w\.]+)'],
    'rust': [r'^\s*use\s+([\w:]+)'],
    'typescript': [r'from\s+[\'"]([^\.][^\'"]*)[\'"]',
                   r'require\([\'"]([^\.][^\'"]*)[\'"]\)'],
    'go': [r'^\s*"([^"]+)"'],
    'c_cpp': [r'^#include\s*[<"]([\w\./]+)[>"]'],
}

LANG_BY_EXT = {
    '.py': 'python', '.rs': 'rust', '.ts': 'typescript', '.tsx': 'typescript',
    '.js': 'typescript', '.mjs': 'typescript', '.go': 'go',
    '.c': 'c_cpp', '.h': 'c_cpp', '.cpp': 'c_cpp', '.cc': 'c_cpp',
    '.hpp': 'c_cpp', '.hh': 'c_cpp',
}


def _lang_for(path):
    """Map a file path to a supported language key, or None."""
    ext = os.path.splitext(path)[1].lower()
    base = os.path.basename(path).lower()
    if base.endswith('.min.js'):
        return None
    return LANG_BY_EXT.get(ext)


def extract_symbols(content, lang):
    """Return [(name, kind, signature)] for a file's content in the given lang."""
    symbols = []
    for line in content.splitlines():
        for pattern, kind in LANG_PATTERNS.get(lang, []):
            m = re.match(pattern, line)
            if m:
                name = m.group(1)
                args = m.group(2) if m.lastindex >= 2 else ''
                sig = '%s(%s)' % (name, ' '.join(args.split()))
                symbols.append((name, kind, sig))
                break
    return symbols


def file_imports(content, lang):
    """Return a set of module tokens a file imports (crude)."""
    imports = set()
    for line in content.splitlines():
        for pattern in IMPORT_PATTERNS.get(lang, []):
            m = re.search(pattern, line)
            if m:
                imports.add(m.group(1).replace(':', '/').replace('.', '/'))
    return imports


def _iter_source_files(root):
    """Yield (relpath, abspath) for source files under root."""
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames
                       if d not in SKIP_DIRS and not d.startswith('.')]
        for fname in filenames:
            if fname in SKIP_EXTS:
                continue
            if any(fname.endswith(e) for e in ('.pyc', '.min.js')):
                continue
            rel = os.path.relpath(os.path.join(dirpath, fname), root)
            if _lang_for(rel) is None:
                continue
            yield rel, os.path.join(dirpath, fname)


def git_rev(root):
    """Return the worktree's HEAD commit, or an mtime-based fallback."""
    try:
        out = subprocess.run(
            ['git', '-C', root, 'rev-parse', 'HEAD'],
            capture_output=True, text=True, timeout=5)
        if out.returncode == 0:
            return out.stdout.strip()
    except Exception:
        pass
    newest = max((os.path.getmtime(os.path.join(dp, f))
                  for dp, _, fs in os.walk(root)
                  for f in fs), default=0.0)
    return 'mtime:%d' % int(newest)


def _rank_files(root, files):
    """Rank files by in-degree in the import graph (Aider-style importance)."""
    mod_to_file = {}
    for rel, _ in files:
        mod = os.path.splitext(rel)[0].replace(os.sep, '/')
        mod_to_file[mod] = rel
    in_degree = {rel: 0 for rel, _ in files}
    for rel, abspath in files:
        try:
            content = open(abspath, encoding='utf-8', errors='replace').read()
        except OSError:
            continue
        for mod in file_imports(content, _lang_for(rel)):
            # strip leading module segments (e.g. 'src/foo' -> 'foo')
            candidates = [mod]
            parts = mod.split('/')
            for i in range(1, len(parts)):
                candidates.append('/'.join(parts[i:]))
            for cand in candidates:
                if cand in mod_to_file and cand != rel:
                    in_degree[mod_to_file[cand]] += 1
    return sorted(files, key=lambda rf: (-in_degree[rf[0]], rf[0]))


def build_repo_map(root, budget_tokens=1000, max_files=None):
    """Build a budget-limited Aider-style map. Returns the map text.

    Format per file:
      path/to/file.rs:
        fn foo(a: i32) -> i32
        struct Bar
    """
    files = list(_iter_source_files(root))
    ranked = _rank_files(root, files)
    if max_files:
        ranked = ranked[:max_files]

    lines = []
    used = 0
    for rel, abspath in ranked:
        try:
            content = open(abspath, encoding='utf-8', errors='replace').read()
        except OSError:
            continue
        syms = extract_symbols(content, _lang_for(rel))
        if not syms:
            continue
        entry = [rel + ':']
        for name, kind, sig in syms:
            entry.append('  %s %s' % (kind, sig))
        entry_tokens = sum(estimate_tokens(l) for l in entry) + 1
        if used + entry_tokens > budget_tokens and used > 0:
            break
        used += entry_tokens
        lines.extend(entry)

    return '\n'.join(lines)


def store_repo_map(project_id, root, budget_tokens=1000, max_files=None):
    """Build the repo map and store per-file nodes (versioned by git_rev).

    Returns (map_text, stored_count).
    """
    from . import db
    rev = git_rev(root)
    files = list(_iter_source_files(root))
    ranked = _rank_files(root, files)
    if max_files:
        ranked = ranked[:max_files]

    stored = 0
    for rel, abspath in ranked:
        if _store_one(db, project_id, root, rel, abspath, rev):
            stored += 1
    return build_repo_map(root, budget_tokens, max_files), stored


def store_file_nodes(project_id, root, relfiles):
    """Re-store nodes for specific files (file-edit refresh, P3.4). Returns count."""
    from . import db
    rev = git_rev(root)
    stored = 0
    for rel in relfiles:
        rel = rel.lstrip('/')
        abspath = os.path.join(root, rel)
        if not os.path.isfile(abspath):
            continue
        if _store_one(db, project_id, root, rel, abspath, rev):
            stored += 1
    return stored


def _store_one(db, project_id, root, rel, abspath, rev):
    """Extract one file's symbols and store its versioned node. True if stored."""
    try:
        content = open(abspath, encoding='utf-8', errors='replace').read()
    except OSError:
        return False
    syms = extract_symbols(content, _lang_for(rel))
    if not syms:
        return False
    summary = '\n'.join('%s %s' % (kind, sig) for _, kind, sig in syms)
    db.store_node(project_id, rel, summary, meta={'git_rev': rev})
    return True
