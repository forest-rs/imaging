# Mental Model

`imaging` is a command stream for 2D drawing. The core crate gives names to the commands and defines
how they move through the system; backend crates decide how those commands become pixels or renderer
state.

```text
Application code
      |
      v
   Painter
      |
      v
  PaintSink trait
      |
      +--> record::Scene
      +--> Vello CPU backend
      +--> Vello backend
      +--> validation / diagnostics
```

[`crate::Painter`] is the authoring helper. It is the API most drawing code should use.

[`crate::PaintSink`] is the receiving side. It accepts borrowed drawing commands.

[`crate::record::Scene`] is an owned sink. It stores commands so they can be kept, checked,
replayed, or rendered later.

Backends either receive commands as sinks or consume scenes through rendering traits. The exact
shape depends on the backend crate.

## Streaming And Retained Paths

The streaming path is:

```text
Painter -> PaintSink
```

Use this when the caller can draw directly into the target. A sink might count commands, build a
backend-native scene, validate the stream, or render.

The retained path is:

```text
Painter -> record::Scene -> validate / diagnose / replay / render
```

Use this when the caller needs owned drawing data. That is useful for caching, tests, snapshots,
debugging, and backend-independent storage.

## Terms

- [`crate::Painter`]: helper for emitting drawing commands.
- [`crate::PaintSink`]: trait that receives borrowed drawing commands.
- [`crate::record::Scene`]: owned retained command stream.
- Validation: structural checks for balanced clips, groups, contexts, and referenced data.
- Diagnostics: warnings about suspicious but valid drawing.
- Replay: sending a retained scene into another sink.

## A Few Edges To Know

Backend support is uneven. A scene can be valid and still use a feature a particular backend does
not support yet.

[`crate::record::Scene::validate`] checks structure, not whether the result is visually what you
intended.

Diagnostics are advisory. They are meant to catch likely mistakes without rejecting valid scenes.

The core crate is pre-1.0 and intentionally small, so names and boundaries may still change.
