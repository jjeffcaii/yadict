# yadict

![yadict](logo.jpg)

A fast command-line MDict (`.mdx`) dictionary lookup tool with terminal color rendering.

## Installation

```bash
cargo install --path .
```

## Usage

### Look up a word

```bash
yadict translate <word> [-r <registry>]
```

Queries all `.mdx` dictionaries in `~/.yadict/registry/<registry>/` (default registry: `default`) and prints results separated by dividers. Output is rendered with ANSI colors.

```
$ yadict translate cat
cat /kæt/ CET4 TEM4 ( cats )
› 1
  N-COUNT A cat is a furry animal that has a long tail and sharp claws. Cats are
  often kept as pets.
...
```

Use `-r` to query a different registry:

```bash
yadict translate cat -r mdict
```

### List installed dictionaries

```bash
yadict list [-r <registry>]
```

Lists all `.mdx` files in `~/.yadict/registry/<registry>/` (default: `default`).

```
$ yadict list
金山词霸2009简明英汉汉英词典.mdx
朗文当代英汉双解词典第4版.mdx
```

## Registry

A registry is a YAML file that lists available dictionaries with their download URLs. A built-in `default` registry is included out of the box. Additional registries are stored in `~/.yadict/registries/`.

### Registry file format

```yaml
metadata:
  name: mdict              # registry identifier (used as the directory name under registry/)
  url: https://mdict.org/  # optional: source of the registry

resources:
  - name: 朗文当代英汉双解词典第4版
    category:
      - English-Chinese    # each element is an independent top-level category in the TUI
    mdx: https://example.com/longman.mdx

  - name: 金山词霸2009简明英汉汉英词典
    category:
      - English-Chinese
      - Chinese-English
    mdx: https://example.com/jinshan.mdx
```

`category` is an array of independent top-level categories. A dictionary listed under multiple categories appears under each one in the TUI tree.

### Install a registry

```bash
yadict registry add <path-or-url>
```

Installs a registry from a local YAML file or a remote URL. The file is saved to `~/.yadict/registries/<name>.yaml`. Re-running with the same source overwrites the existing file, which is useful for updating a registry.

```
$ yadict registry add ./registry.yaml
Registry 'mdict' installed (2 dicts) → /Users/you/.yadict/registries/mdict.yaml

$ yadict registry add https://example.com/registry.yaml
Downloading registry from https://example.com/registry.yaml ...
Registry 'mdict' installed (312 dicts) → /Users/you/.yadict/registries/mdict.yaml
```

### Browse and install dictionaries (TUI)

```bash
yadict registry list [query]
```

Opens a full-screen TUI showing all dictionaries from installed registries (including the built-in `default`), organized as a collapsible category tree. Select one or more dictionaries and press `Enter` to download them.

```
$ yadict registry list
$ yadict registry list english   # pre-filter by name or category
```

Dictionaries are downloaded in parallel and saved to `~/.yadict/registry/<registry>/<name>.mdx`.

Key bindings in the TUI:

| Key | Action |
|-----|--------|
| `↑` / `↓` / `j` / `k` | Move cursor |
| `l` / `→` | Expand category |
| `h` / `←` | Collapse category |
| `Space` | Toggle selection (entry) / expand-collapse (category) |
| `a` | Select / deselect all visible entries |
| `/` | Enter search mode — live filters name and category |
| `Esc` | Clear search and return to tree view |
| `Enter` | Download all selected dictionaries and quit |
| `q` | Quit without downloading |

### Search registries (plain output)

```bash
yadict registry search <query>
```

Prints all dictionaries whose name, category, or registry name matches the query (case-insensitive).

```
$ yadict registry search longman
[English-Chinese] 朗文当代英汉双解词典第4版  (default)
```

### Reload registries

```bash
yadict registry refresh
```

Reloads all entries from the installed registry YAML files on disk. Useful after manually editing a registry file. The built-in `default` registry is always included.

## Data directory

By default, yadict stores data under `~/.yadict/`. Set the `YADICT_HOME` environment variable to use a different location:

```bash
export YADICT_HOME=/data/yadict
yadict translate hello
```

## Directory layout

```
$YADICT_HOME/
├── registry/                # installed .mdx dictionary files, organized by registry
│   ├── default/
│   │   ├── 朗文当代英汉双解词典第4版.mdx
│   │   └── 金山词霸2009简明英汉汉英词典.mdx
│   └── mdict/
│       └── collins.mdx
├── registries/              # user-installed registry YAML files
│   └── mdict.yaml
└── cache.tsv                # URL → local path index (managed automatically)
```
