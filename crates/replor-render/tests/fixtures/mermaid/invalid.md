# Invalid syntax

The diagram below cannot be parsed and falls back to its source.

```mermaid
graph LR
  A -->
  --> B
  [[[
```

Text after the diagram still renders.
