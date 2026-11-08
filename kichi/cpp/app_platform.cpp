#include "app_platform.h"

#include <QtGui/QColor>
#include <QtGui/QGuiApplication>
#include <QtGui/QPalette>

#include <algorithm>
#include <cmath>

namespace {
QColor withAlpha(QColor color, double alpha)
{
    color.setAlphaF(alpha);
    return color;
}

QPalette makePalette(bool dark)
{
    QPalette palette;

    const QColor window = dark ? QColor::fromRgb(0x24, 0x24, 0x24)
                               : QColor::fromRgb(0xf2, 0xf2, 0xf2);
    const QColor base = dark ? QColor::fromRgb(0x1e, 0x1e, 0x1e)
                             : QColor::fromRgb(0xff, 0xff, 0xff);
    const QColor alternateBase = dark ? QColor::fromRgb(0x26, 0x26, 0x26)
                                      : QColor::fromRgb(0xf7, 0xf7, 0xf7);
    const QColor button = dark ? QColor::fromRgb(0x2e, 0x2e, 0x2e)
                               : QColor::fromRgb(0xf4, 0xf4, 0xf4);
    const QColor text = dark ? QColor::fromRgb(0xf6, 0xf6, 0xf6)
                             : QColor::fromRgb(0x1f, 0x1f, 0x1f);
    const QColor mid = dark ? QColor::fromRgb(0x55, 0x55, 0x55)
                            : QColor::fromRgb(0xb8, 0xb8, 0xb8);
    const QColor midlight = dark ? QColor::fromRgb(0x33, 0x33, 0x33)
                                 : QColor::fromRgb(0xe0, 0xe0, 0xe0);
    const QColor darkColor = dark ? QColor::fromRgb(0x10, 0x10, 0x10)
                                  : QColor::fromRgb(0x9f, 0x9f, 0x9f);
    const QColor light = dark ? QColor::fromRgb(0x3a, 0x3a, 0x3a)
                              : QColor::fromRgb(0xff, 0xff, 0xff);
    const QColor highlight = QColor::fromRgb(0x35, 0x84, 0xe4);
    const QColor link = dark ? QColor::fromRgb(0x78, 0xae, 0xff)
                             : QColor::fromRgb(0x1a, 0x73, 0xe8);
    const QColor visitedLink = dark ? QColor::fromRgb(0xc3, 0x9b, 0xff)
                                    : QColor::fromRgb(0x7b, 0x3f, 0xb5);
    const QColor disabledText = withAlpha(text, 0.4);

    palette.setColor(QPalette::Window, window);
    palette.setColor(QPalette::WindowText, text);
    palette.setColor(QPalette::Base, base);
    palette.setColor(QPalette::AlternateBase, alternateBase);
    palette.setColor(QPalette::Text, text);
    palette.setColor(QPalette::Button, button);
    palette.setColor(QPalette::ButtonText, text);
    palette.setColor(QPalette::BrightText, dark ? QColor::fromRgb(0xff, 0x6b, 0x6b)
                                                : QColor::fromRgb(0xc0, 0x39, 0x2b));
    palette.setColor(QPalette::Light, light);
    palette.setColor(QPalette::Midlight, midlight);
    palette.setColor(QPalette::Dark, darkColor);
    palette.setColor(QPalette::Mid, mid);
    palette.setColor(QPalette::Shadow, Qt::black);
    palette.setColor(QPalette::Highlight, highlight);
    palette.setColor(QPalette::HighlightedText, Qt::white);
    palette.setColor(QPalette::Link, link);
    palette.setColor(QPalette::LinkVisited, visitedLink);
    palette.setColor(QPalette::ToolTipBase, button);
    palette.setColor(QPalette::ToolTipText, text);
    palette.setColor(QPalette::PlaceholderText, withAlpha(text, dark ? 0.55 : 0.50));
    palette.setColor(QPalette::Accent, highlight);

    palette.setColor(QPalette::Disabled, QPalette::WindowText, disabledText);
    palette.setColor(QPalette::Disabled, QPalette::Text, disabledText);
    palette.setColor(QPalette::Disabled, QPalette::ButtonText, disabledText);
    palette.setColor(QPalette::Disabled, QPalette::HighlightedText, disabledText);
    palette.setColor(QPalette::Disabled, QPalette::Highlight, highlight);

    return palette;
}

bool isDark(const QPalette &palette)
{
    return palette.color(QPalette::Base).lightnessF() < 0.5;
}

double linearChannel(double value)
{
    return value <= 0.04045 ? value / 12.92
                            : std::pow((value + 0.055) / 1.055, 2.4);
}

double relativeLuminance(const QColor &color)
{
    return 0.2126 * linearChannel(color.redF())
        + 0.7152 * linearChannel(color.greenF())
        + 0.0722 * linearChannel(color.blueF());
}

// 深色强调色配白字，浅色强调色配黑字。阈值取 0.5：GNOME 蓝 (#3584E4)、
// KDE 绿这类中等亮度颜色用白字，黄色 / 浅绿这类亮色用黑字。
QColor accentForeground(const QColor &accent)
{
    return relativeLuminance(accent) > 0.5 ? Qt::black : Qt::white;
}

double clamp01(double value)
{
    return std::clamp(value, 0.0, 1.0);
}
} // namespace

extern "C" void kichi_set_quit_on_last_window_closed(bool enabled)
{
    QGuiApplication::setQuitOnLastWindowClosed(enabled);
}

extern "C" void kichi_apply_appearance(int scheme, int hasAccent,
                                       double accentRed, double accentGreen,
                                       double accentBlue)
{
    // 记住「平台原本给的调色板」，而不是每次都在上一次结果上叠加。
    // 如果平台主题（例如 GTK 深色切换）在我们之外改了 palette，就把它当成新的
    // 起点，避免恢复时把外部变化一起丢掉。
    static bool captured = false;
    static bool hasApplied = false;
    static QPalette original;
    static QPalette applied;

    const QPalette current = QGuiApplication::palette();
    if (!captured) {
        original = current;
        captured = true;
    } else if (hasApplied && current != applied) {
        original = current;
    }

    QPalette palette = original;

    if (scheme >= 0 && isDark(original) != (scheme != 0))
        palette = makePalette(scheme != 0);

    if (hasAccent) {
        const QColor accent = QColor::fromRgbF(clamp01(accentRed),
                                               clamp01(accentGreen),
                                               clamp01(accentBlue));
        const QColor foreground = accentForeground(accent);

        palette.setColor(QPalette::Highlight, accent);
        palette.setColor(QPalette::HighlightedText, foreground);
        palette.setColor(QPalette::Disabled, QPalette::Highlight, accent);
        palette.setColor(QPalette::Disabled, QPalette::HighlightedText, foreground);
        palette.setColor(QPalette::Accent, accent);
        palette.setColor(QPalette::Link, accent);
    }

    if (palette == current) {
        applied = palette;
        hasApplied = true;
        return;
    }

    QGuiApplication::setPalette(palette);
    applied = palette;
    hasApplied = true;
}
