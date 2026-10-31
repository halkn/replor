# 日本語ラベルの Mermaid（P0 Spike S1・S2）

## flowchart

```mermaid
flowchart LR
  A[開始] --> B{レビュー待ち？}
  B -->|承認| C[マージ]
  B -->|差し戻し| D[修正する]
  D --> B
  C --> E[完了 Done]
```

## sequenceDiagram

```mermaid
sequenceDiagram
  participant 作成者
  participant レビュアー
  作成者->>レビュアー: PR を作成しました
  レビュアー-->>作成者: コメント（3件）
  作成者->>レビュアー: 修正済み
  レビュアー-->>作成者: 承認 ✅
```

## erDiagram

```mermaid
erDiagram
  顧客 ||--o{ 注文 : 発注する
  注文 ||--|{ 明細 : 含む
  顧客 {
    string name "氏名"
    string email "メール"
  }
```

## Azure Wiki 形式

::: mermaid
graph TD
  W[Wiki ページ] --> X[添付画像 .attachments]
  W --> Y[子ページ]
:::

## 曖昧幅の文字（○・①・→）

```mermaid
flowchart LR
  P[① 受付○] --> Q[② 処理→完了]
```
