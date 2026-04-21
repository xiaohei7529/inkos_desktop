---
version: "1.0"
protagonist:
  name: ""
  personalityLock: []
  behavioralConstraints: []
genreLock:
  primary: ""
  forbidden: []
prohibitions: []
chapterTypesOverride: []
fatigueWordsOverride: []
additionalAuditDimensions: []
enableFullCastTracking: false
---

## 叙事视角

（描述本书叙事视角和风格）

## 核心冲突驱动

（描述本书的核心矛盾和驱动力）

## 禁止的元叙事元素

审计时必须检查以下元素**不得出现在正文章节中**：

| 类型 | 示例 | 说明 |
| --- | --- | --- |
| 伏笔 ID | HOOK_001、HOOK_003 等 | 伏笔追踪是系统内部术语，角色不会这样思考 |
| 章节类型 | "探索章"、"交易章"等 | 写作指令，不应出现在正文 |
| 作者视角注释 | "这是伏笔"、"备选故事"等 | 角色无法感知叙事结构 |
| 系统术语 | "audit"、"manifest"、"state"等 | InkOS 系统内部用语 |
| 打破第四墙 | 暗示读者、暗示故事性质 | 破坏沉浸感 |
| 章节引用 | "第 X 章"、"上一章"、"第三章末尾"等 | 角色无法感知章节结构，应使用"那次"、"之前"等自然指代 |

**正确做法：** 所有系统信息必须转化为角色的自然思考。例如：

- ❌ "这就是 HOOK_003 的答案" → ✅ "心里有数了"
- ❌ "沾染晶核气息是备选故事" → ✅ "身上带点晶核气息，能应付查验"
