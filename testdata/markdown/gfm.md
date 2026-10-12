# GFM 表示評価

Intro paragraph with a [relative link](./other.md), a [reference link][ref] and <https://example.com/auto>.

## 表（Table）

| 名前 | 説明 | 状態 |
|:---|---|--:|
| 日本語の長いセル内容をここに書いて折り返しを確認する | `code` を含む | ✅ |
| Emoji 😀👍🏽 | 全角ＡＢＣ１２３ | 100 |
| 結合文字 é / か゚ | 半角ｶﾅ | 3 |

## Task List

- [x] 完了したタスク
- [ ] 未完了のタスク
  - [ ] ネストしたタスク

## Alert

> [!NOTE]
> GitHub の Alert 構文。glamour では通常の引用として表示される想定。

> [!WARNING]
> 警告の Alert。

## 脚注

脚注付きの文[^1]と、もう一つ[^note]。

[^1]: 一つ目の脚注。
[^note]: 名前付き脚注。

## 日本語長文

吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。何でも薄暗いじめじめした所でニャーニャー泣いていた事だけは記憶している。吾輩はここで始めて人間というものを見た。しかもあとで聞くとそれは書生という人間中で一番獰悪な種族であったそうだ。

## コード

```go
func main() {
	fmt.Println("こんにちは")
}
```

## 重複見出し

## 重複見出し

### Data Flow & API (v2)

[back to top](#gfm-表示評価) / [dup](#重複見出し-1) / [data](#data-flow--api-v2)

Control chars: before[2Jafter and OSC ]8;;https://evil.examplelink]8;;

[ref]: https://example.com/ref "Ref Title"
