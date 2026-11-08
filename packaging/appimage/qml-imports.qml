// 仅用于 linuxdeploy-plugin-qt 扫描。
// 应用的 QML 已编进 qrc，扫描器看不到运行时导入；
// 这里显式列出 KDE 控件风格、Fusion 和 Kirigami 模块，确保它们被 AppImage 带上。
import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Fusion as Fusion
import org.kde.desktop as Desktop
import org.kde.desktop.private as DesktopPrivate
import org.kde.kirigami as Kirigami
import org.kde.kirigami.controls as KirigamiControls
import org.kde.kirigami.dialogs as KirigamiDialogs
import org.kde.kirigami.forms as KirigamiForms
import org.kde.kirigami.layouts as KirigamiLayouts
import org.kde.kirigami.platform as KirigamiPlatform
import org.kde.kirigami.primitives as KirigamiPrimitives
import org.kde.kirigami.templates as KirigamiTemplates
import org.kde.qqc2desktopstyle.private as DesktopStylePrivate

Item {}
