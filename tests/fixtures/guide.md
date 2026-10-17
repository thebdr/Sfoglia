# Sfoglia guide

Sfoglia opens *anything*. This guide shows the rendered Markdown with _emphasis_, **strong**, ~~strike~~ and `code`.

## Install

1. Install Rust
2. Clone the repository
   - with git
   - or as a zip
3. Run `cargo run`

## Features

- [x] text and hex
- [ ] office documents

> Nothing is true until reality shows it.

---

### Formats

| Format | Read | Write |
| ------ | ---- | ----- |
| text   | yes  | yes   |
| hex    | yes  | no    |
| images | yes  | save-as |

## Code

```rust
fn main() {
    println!("hello");
}
```

```
plain text
```

## Links and images

The [site](https://example.com), the [readme][readme], the [install section](#install), a note[^1] and the dot.

![dot](dot.png)

![dot too][dot]

[^1]: A footnote.

[readme]: ../../README.md
[dot]: dot.png
