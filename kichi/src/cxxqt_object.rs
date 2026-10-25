//! 桥接对象：Rust 侧暴露给 QML 的全局信息。
//!
//! S1 只放一个 `version` 属性，证明 cxx-qt 桥接链路端到端可用；
//! 后续会话状态（令牌、任务轮询等）一律放 `session` 模块，QML 只消费数据。

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        // 告诉 CXX-Qt 生成名为 Kichi 的 QObject 类，基于 Rust 结构体 KichiRust
        #[qobject]
        #[qml_element]
        #[qproperty(QString, version)]
        #[namespace = "kichi"]
        type Kichi = super::KichiRust;
    }
}

use cxx_qt_lib::QString;

/// QObject `Kichi` 的 Rust 载体。
#[derive(Debug)]
pub struct KichiRust {
    version: QString,
}

impl Default for KichiRust {
    fn default() -> Self {
        Self {
            version: QString::from(env!("CARGO_PKG_VERSION")),
        }
    }
}
