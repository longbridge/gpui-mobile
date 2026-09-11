## gpui-kit · Markdown

**Bold**, *italic*, ***both***, ~~deleted~~, `inline code`, and a [link](https://gpui-kit.com). 🌿

> **Note:** Rich text inside a quote, with *emphasis* and `code`.

- **Nested list**
  - Mix *styles* and `values`.
  - Wrap naturally on a small screen.

```rust
let view = TextView::markdown("demo", text)
    .selectable(true);
```

| Feature | Example | State |
| :--- | :---: | ---: |
| **Text** | *Styled* | Ready |
| `Code` | `42` | Done |
| Link | [Docs](https://gpui-kit.com) | Open |

---

### More cases

- [x] Completed task
- [ ] Pending task

1. An ordered item with **bold** text.
2. A second item with a nested quote:

   > Nested blocks keep their indentation.

Escaped: \*literal\*, \[brackets\], &lt;tag&gt;, &amp;. Backticks: `` `value` ``.

### Nested quote

> First level.
>
> > Second level with **bold** and *italic*.
>
> Back to the first level.

### Code inside a fence

````markdown
```json
{"enabled": true, "items": [1, 2, 3]}
```
````

### Wrapping table

| Case | Content |
| --- | --- |
| Long cell | A longer description tests wrapping within a narrow table cell. |
| Mixed | **Bold**, *italic*, ~~removed~~, `code` |
| Pipe | Left \| right |
| Empty | |

### Line breaks

A hard break follows here.  
This starts a new line.

A long **bold phrase with enough words to cross a line boundary**, followed by *italic text* and `inline_code`.

#### Heading four

##### Heading five

###### Heading six

**End of document.**
