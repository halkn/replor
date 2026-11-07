# Malformed extensions

A fenced block containing macro-like text, which must stay literal:

```text
[[_TOC_]]
[[_TOSP_]]
::: mermaid
graph LR
:::
```

A bare `:::` on its own is ordinary text:

:::

An unknown container keeps its body:

::: not-a-real-container
this text must survive
:::

A container that is never closed ends at the blank line, so the rest of the
document stays readable:

::: mermaid
graph LR
  A --> B

Text after the unterminated container.
