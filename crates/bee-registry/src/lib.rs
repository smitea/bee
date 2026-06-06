//! `bee-registry` — Bee 注册中心。
//!
//! 管理 Plugin 加载 / ABI 校验 / 哈希校验 / 网络同步 / 路由表广播。
//! Registry 是 trait,具体实现可插拔 (本地、etcd 风格、内存测试桩)。
//!
//! S00 阶段仅占位;S19 起实现 [`PluginManager`] 与 [`NetworkSync`]。

pub struct Registry;
