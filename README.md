# `toy_pool`

Items in the pool will be reference-counted with strong `Handle` s. When no `Handle` is referring to an item, it can be removed on synchronization, or you can handle it manually.

## Motivation

I'm going to re-implement [my-game] 's scene graph with this crate.

[my-game]: https://github.com/toyboot4e/snowrl

## Optional `igri` support

[igri] is ImGUI runtime inspector.

[igri]: https://github.com/toyboot4e/igri

