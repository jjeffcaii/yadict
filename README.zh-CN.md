# yadict

![yadict](logo.jpg)

快速的命令行 MDict（`.mdx`）词典查询工具，支持终端彩色渲染。

## 安装

```bash
cargo install --path .
```

## 命令

### `translate`（别名 `t`）

```bash
yadict translate <单词> [--markdown] [--html] [-r <registry>]
yadict t <单词>
```

查询 `~/.yadict/registry/<registry>/` 下所有 `.mdx` 词典（默认 registry：`default`），以 ANSI 彩色渲染输出结果，多个词典的结果之间以分隔线隔开。

```
$ yadict t cat
cat
n.  猫；猫科动物
...
```

参数说明：

| 参数 | 说明 |
|------|------|
| `--markdown` | 输出原始 Markdown，不做终端渲染 |
| `--html` | 输出词典条目的原始 HTML |
| `-r <名称>` | 指定 registry（默认：`default`） |

### `list`

```bash
yadict list [-r <registry>]
```

列出 `~/.yadict/registry/<registry>/` 下已安装的所有词典（默认：`default`）。

```
$ yadict list
金山词霸2009简明英汉汉英词典
朗文当代英汉双解词典第4版
```

### `remove`

```bash
yadict remove <名称> [-r <registry>]
```

删除已安装的词典文件及其缓存记录。

```
$ yadict remove 朗文当代英汉双解词典第4版
Removed '朗文当代英汉双解词典第4版' from registry 'default'.
```

### `registry`（别名 `r`）

管理远程词典索引。

#### `registry list [关键词]`

```bash
yadict registry list
yadict r list english
```

打开全屏 TUI，以可折叠分类树的形式展示所有可用词典。已安装的词典会被预先勾选并标记 `✓`。

- **勾选**未安装的条目 → 下载
- **取消勾选**已安装的条目 → 删除
- 按 `Enter` 进入确认弹窗，再次确认后执行操作
- 若未做任何改动，直接静默退出

下载并行进行，带实时进度条。

按键说明：

| 按键 | 功能 |
|------|------|
| `↑` / `↓` / `j` / `k` | 移动光标 |
| `→` / `l` | 展开分类 |
| `←` / `h` | 折叠分类 |
| `Space` | 切换选中（条目）/ 展开折叠（分类） |
| `a` | 全选 / 全不选当前可见条目 |
| `/` | 进入搜索模式，实时按名称或分类过滤 |
| `Esc` | 清除搜索，返回分类树视图 |
| `Enter` | 确认，进入下载/删除确认弹窗 |
| `q` / `Esc` | 不做任何改动直接退出 |

#### `registry search <关键词>`

```bash
yadict registry search longman
```

在所有 registry 条目中按名称、分类或 registry 名进行纯文本搜索（大小写不敏感）。

```
$ yadict registry search longman
[English-Chinese] 朗文当代英汉双解词典第4版  (default)
```

#### `registry add <路径或URL>`

```bash
yadict registry add ./my-registry.yaml
yadict registry add https://example.com/registry.yaml
```

从本地 YAML 文件或远程 URL 安装一个 registry。内置的 `default` registry 无需此步骤即可直接使用。

#### `registry refresh`

```bash
yadict registry refresh
```

从磁盘重新加载所有 registry 条目，手动编辑 YAML 文件后可使用此命令刷新。

## Registry 文件格式

```yaml
metadata:
  name: default         # registry 标识符，同时作为 registry/ 下的子目录名
  url: https://mdict.org/

resources:
  - name: 朗文当代英汉双解词典第4版
    category:
      - English-Chinese  # 每个元素对应 TUI 中一个独立的顶级分类
    mdx: https://example.com/longman.mdx

  - name: 金山词霸2009简明英汉汉英词典
    category:
      - English-Chinese
      - Chinese-English
    mdx: https://example.com/jinshan.mdx
```

一个词典可归属于多个分类，在 TUI 分类树中会出现在每个对应的分类下。

## 数据目录

默认数据目录为 `~/.yadict/`，可通过环境变量 `YADICT_HOME` 覆盖：

```bash
export YADICT_HOME=/data/yadict
yadict t hello
```

目录结构：

```
$YADICT_HOME/
├── registry/           # 已安装的 .mdx 文件，按 registry 名分目录存放
│   ├── default/
│   │   ├── 金山词霸2009简明英汉汉英词典.mdx
│   │   └── 朗文当代英汉双解词典第4版.mdx
│   └── mdict/
│       └── collins.mdx
├── registries/         # 用户安装的 registry YAML 文件
│   └── mdict.yaml
└── cache.tsv           # URL → 本地路径索引（自动维护）
```
