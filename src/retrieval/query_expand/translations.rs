use std::collections::HashMap;
use std::sync::LazyLock;

const CJK_EN_PAIRS: &[(&str, &str)] = &[
    ("encrypt", "加密"),
    ("encryption", "加密"),
    ("database", "数据库"),
    ("db", "数据库"),
    ("api", "接口"),
    ("interface", "接口"),
    ("hook", "钩子"),
    ("hooks", "钩子"),
    ("trait", "特征"),
    ("adapter", "适配器"),
    ("benchmark", "基准测试"),
    ("eval", "评测"),
    ("bench", "基准测试"),
    ("compression", "压缩"),
    ("compaction", "压缩"),
    ("timeout", "超时"),
    ("worker", "工作线程"),
    ("memory", "记忆"),
    ("memories", "记忆"),
    ("search", "搜索"),
    ("retrieval", "检索"),
    ("query", "查询"),
    ("fts", "全文搜索"),
    ("fts5", "全文搜索"),
    ("tokenizer", "分词器"),
    ("tokenize", "分词"),
    ("prompt", "提示词"),
    ("publish", "发布"),
    ("posting", "发帖"),
    ("deploy", "部署"),
    ("deployment", "部署"),
    ("config", "配置"),
    ("configuration", "配置"),
    ("port", "端口"),
    ("session", "会话"),
    ("scope", "作用域"),
    ("global", "全局"),
    ("summary", "摘要"),
    ("format", "格式"),
    ("server", "服务器"),
    ("service", "服务"),
    ("twitter", "推特"),
    ("promote", "提升"),
    ("auto", "自动"),
    ("automatic", "自动"),
    ("performance", "性能"),
    ("video", "视频"),
    ("quality", "质量"),
    ("cost", "成本"),
    ("context", "上下文"),
    ("competitive", "竞品"),
    ("comparison", "对比"),
    ("optimization", "优化"),
    ("optimize", "优化"),
    ("preference", "偏好"),
    ("sharing", "共享"),
    ("hang", "挂起"),
    ("stuck", "卡住"),
    ("architecture", "架构"),
    ("design", "设计"),
    ("rules", "规则"),
    ("rule", "规则"),
    ("failure", "失败"),
    ("failed", "失败"),
    ("error", "错误"),
    ("task", "任务"),
    ("job", "任务"),
    ("mechanism", "机制"),
    ("implementation", "实现"),
    ("implement", "实现"),
    ("cross-project", "跨项目"),
];

pub(super) static CJK_EN_TRANSLATIONS: LazyLock<HashMap<&'static str, Vec<&'static str>>> =
    LazyLock::new(|| {
        let mut translations: HashMap<&str, Vec<&str>> = HashMap::new();
        for (english, cjk) in CJK_EN_PAIRS {
            insert_translation(&mut translations, english, cjk);
            insert_translation(&mut translations, cjk, english);
        }
        translations
    });

fn insert_translation(
    translations: &mut HashMap<&'static str, Vec<&'static str>>,
    from: &'static str,
    to: &'static str,
) {
    let values = translations.entry(from).or_default();
    if !values.contains(&to) {
        values.push(to);
    }
}
