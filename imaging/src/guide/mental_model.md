# Mental model

`imaging` defines a common vocabulary for 2D drawing. A [`crate::Painter`] emits borrowed
commands into a [`crate::PaintSink`]. The sink can be a renderer, a backend-native recorder,
a retained scene, or a tool that inspects commands.

```text
Painter → PaintSink → renderer or other streaming consumer
                    → record::Scene → validate, diagnose, or replay
```

## Stream commands

Give a sink to [`crate::Painter::new`] to draw directly into that target. A
[`crate::validation::ValidatingSink`] can check commands while forwarding them to another sink.
A [`crate::diagnostics::DiagnosingSink`] can report advisory findings as commands pass through.

## Keep a recording

Use a [`crate::record::Scene`] as the sink to keep an owned recording for caching, tests,
inspection, or rendering later. [`crate::record::Scene::validate`] checks the scene's structure
and payloads, including balanced scopes and valid resource references.
[`crate::record::Scene::diagnose`] reports suspicious patterns, such as an empty clip or a fully
transparent draw. [`crate::record::replay`] sends the recording to another sink.

A valid scene can still use a drawing feature that its renderer does not support. Consult the
renderer crate's documentation for capabilities and resource requirements.
