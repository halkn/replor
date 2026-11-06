# Code blocks

A fenced block with a known language:

```rust
// Entry point.
fn main() {
    let answer: u32 = 42;
    let greeting = "hello";
    println!("{greeting} {answer}");
}
```

A language the highlighter does not know stays uniform:

```no-such-language
value = 1 // not a comment here
```

A block with no language at all:

```
plain text, indented
    and kept as written
```

Tabs expand to four columns:

```python
def f():
	return "tab indented"
```

Long lines are never wrapped; the pager scrolls instead:

```json
{"a": 1, "b": [true, false, null], "c": "a fairly long string value to push past the width"}
```
