# 日本語の属性名

merman 0.7.0 はこの図で panic し、文書全体が表示されなかった。

```mermaid
erDiagram
  顧客 ||--o{ 注文 : 発注する
  顧客 {
    string 氏名
    string メール
  }
```

図の後の段落も表示される。
