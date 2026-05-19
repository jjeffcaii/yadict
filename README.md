# yadict

![yadict](logo.jpg)

A fast command-line MDict (`.mdx`) dictionary lookup tool with terminal color rendering.

## Installation

```bash
cargo install --path .
```

## Commands

### `translate` (`t`)

```bash
yadict translate <word> [--markdown] [--html] [-r <registry>]
yadict t <word>
```

Queries all `.mdx` dictionaries in `~/.yadict/registry/<registry>/` (default: `default`) and prints results with ANSI color rendering. Multiple dictionary results are separated by dividers.

```
$ yadict t cat
cat
n.  猫；猫科动物
...
```

Flags:

| Flag | Description |
|------|-------------|
| `--markdown` | Print raw Markdown instead of rendered output |
| `--html` | Print raw HTML from the dictionary entry |
| `-r <name>` | Query a different registry (default: `default`) |

### `list`

```bash
yadict list [-r <registry>]
```

Lists all installed dictionaries in `~/.yadict/registry/<registry>/` (default: `default`).

```
$ yadict list
金山词霸2009简明英汉汉英词典
朗文当代英汉双解词典第4版
```

### `remove`

```bash
yadict remove <name> [-r <registry>]
```

Removes an installed dictionary and its cache entry.

```
$ yadict remove 朗文当代英汉双解词典第4版
Removed '朗文当代英汉双解词典第4版' from registry 'default'.
```

### `registry` (`r`)

Manage the remote dictionary index.

#### `registry list [query]`

```bash
yadict registry list
yadict r list english
```

Opens a full-screen TUI showing all available dictionaries organized as a collapsible category tree. Already-installed dictionaries are pre-selected and marked with `✓`.

- **Check** an uninstalled entry → download it
- **Uncheck** an installed entry → delete it
- Press `Enter` to review a confirmation dialog, then confirm or cancel
- If nothing changed, exits silently without prompting

Downloads run in parallel with progress bars.

Key bindings:

| Key | Action |
|-----|--------|
| `↑` / `↓` / `j` / `k` | Move cursor |
| `→` / `l` | Expand category |
| `←` / `h` | Collapse category |
| `Space` | Toggle selection (entry) / expand-collapse (category) |
| `a` | Select / deselect all visible entries |
| `/` | Enter search mode — live-filters by name and category |
| `Esc` | Clear search and return to tree view |
| `Enter` | Confirm and proceed to download/remove |
| `q` / `Esc` | Quit without changes |

#### `registry search <query>`

```bash
yadict registry search longman
```

Plain-text search across all registry entries by name, category, or registry name (case-insensitive).

```
$ yadict registry search longman
[English-Chinese] 朗文当代英汉双解词典第4版  (default)
```

#### `registry add <path-or-url>`

```bash
yadict registry add ./my-registry.yaml
yadict registry add https://example.com/registry.yaml
```

Installs a registry from a local YAML file or a remote URL. The built-in `default` registry is always available without this step.

#### `registry refresh`

```bash
yadict registry refresh
```

Reloads all registry entries from disk. Useful after manually editing a registry YAML file.

## Registry file format

```yaml
metadata:
  name: default         # registry identifier — used as the directory name under registry/
  url: https://mdict.org/

resources:
  - name: 朗文当代英汉双解词典第4版
    category:
      - English-Chinese  # each element is an independent top-level category in the TUI
    mdx: https://example.com/longman.mdx

  - name: 金山词霸2009简明英汉汉英词典
    category:
      - English-Chinese
      - Chinese-English
    mdx: https://example.com/jinshan.mdx
```

A dictionary listed under multiple categories appears under each one in the TUI tree.

## Data directory

By default, yadict stores data under `~/.yadict/`. Override with `YADICT_HOME`:

```bash
export YADICT_HOME=/data/yadict
yadict t hello
```

```
$YADICT_HOME/
├── registry/           # installed .mdx files, organized by registry name
│   ├── default/
│   │   ├── 金山词霸2009简明英汉汉英词典.mdx
│   │   └── 朗文当代英汉双解词典第4版.mdx
│   └── mdict/
│       └── collins.mdx
├── registries/         # user-installed registry YAML files
│   └── mdict.yaml
└── cache.tsv           # URL → local path index (managed automatically)
```
