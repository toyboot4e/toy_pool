# `toy_pool`

Items in the pool are reference-counted by strong `Handle` s. When no `Handle` is referring to an item, it can be removed with `Pool::sync_refcounts_and_invalidate`, or you can handle removal with `Pool::sync_refcounts` and `Pool::invalidate_unreferenced`.

## Motivation

I'm going to re-implement [my-game] 's scene graph with this crate.

[my-game]: https://github.com/toyboot4e/snowrl

