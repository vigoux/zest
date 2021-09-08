# zest

A personal note management `CLI` tool.

## Quickstart

1. Install `zest`: `cargo install zest`
2. Create your config file:
```yaml
# The path is ~/.config/zest/config.yml

# List the paths you use here
paths:
  - ~/notes/
```
3. Add notes, the format is simple: markdown + metadata on top
```
---
# Metadata is in yaml

# The tags you can to apply on this file
tags:
  - foo
  - bar
---

# Title

It is recommended to add a title to your file
You can link to other notes with normal markdown links: [mylink](myfile)
```
4. Run `zest init`
5. Search with `zest search`

## Searching

`zest` queries are just `tantivy` queries, with the following fields
that you can query:
- `file`: the file containing the note (any part of the full path)
- `tag`: the tags use for this note
- `ref`: outgoing refs of the note
- `title`: what is in the title
- `content`: what is in the content

By default, search terms apply to the `title` and `content` fields.

### Examples

Notes containing `foo`:
```
zest search foo
```

Notes tagged `foo`:
```
zest search tag:foo
```

Notes refering to `foo`:
```
zest search ref:foo
```

Notes refering to `foo` and tagged `bar`:
```
zest search ref:foo AND tag:bar
```

## Philosophy

`zest` is a note management tool (or a knowledge base manager) that
allows you to create, search and display notes in your knowledge
database.

It is meant to be used in a Zettelkasten-like workflow, where each
note is it's own file (the full path is the note's id). You can then
tag this file using metadata in your file, and add links to other
notes.

The `CLI` design is inspired by `notmuch` a mail indexer, which does
only mail indexing and querying, just like zest.

Designed with scripting in mind, it is easy to add support for zest to
your editor.

In summary, here are the design goals:
- Searching has to be fast
- Inserting and updating notes can be a bit longer
- Writing notes should use markdown

## Performances

`zest` relies on `tantivy` to perform the indexing and searching part.
Because of that, the indexing will greatly vary depending on the speed
of your SSD/HDD.

On my end, with a terribly slow HDD, indexing my whole database (not
really big yet), takes the monstruous time of 10s.

Querying is generally really fast, I can gather all the references to
the current file in something like 0.1s.

## TODO

- [ ] Add more query possibilities, currently only support `tantivy`
      queries, would like to have nested queries
- [ ] Add editor supports (Nvim support is WIP)
