# `velato_imaging`

Velato rendering through the `imaging` command stream.

This crate:

- re-exports [`velato`]
- provides [`ImagingSink`] for translating Velato draw commands into `imaging::PaintSink`
- provides [`RendererExt`] helpers for appending a composition into any imaging sink or building an
  `imaging::record::Scene`
