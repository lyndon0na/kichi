#include "mpv_video_item.h"

#include "embed_ffi.h"
#include "wayland_video_surface.h"

#include <QDebug>
#include <QQuickWindow>

MpvVideoItem::MpvVideoItem(QQuickItem *parent)
    : QQuickItem(parent)
{
    m_poll.setInterval(10);
    m_poll.setTimerType(Qt::PreciseTimer);
    connect(&m_poll, &QTimer::timeout, this, &MpvVideoItem::poll);
    connect(this, &QQuickItem::windowChanged, this, [this](QQuickWindow *window) {
        if (m_native) {
            m_native->setFrameCallback({});
            m_native.reset();
        }
        if (window) {
            // 窗口的 wl_surface 要等 Qt 创建平台窗口后才存在，延后一拍。
            QTimer::singleShot(0, this, [this] { setupNative(); });
        } else {
            m_hasFrame = false;
            emit hasFrameChanged();
        }
    });
}

MpvVideoItem::~MpvVideoItem()
{
    m_poll.stop();
    if (m_native) {
        m_native->setFrameCallback({});
        m_native.reset();
    }
}

void MpvVideoItem::setupNative()
{
    if (m_native)
        return;
    QQuickWindow *win = window();
    if (!win)
        return;

    auto native = std::make_unique<WaylandVideoSurface>(win);
    if (!native->available()) {
        setError(QStringLiteral("内嵌播放需要 Wayland 合成器支持 dmabuf / subsurface"));
        return;
    }
    m_native = std::move(native);
    m_native->setFrameCallback([this] {
        const bool frame = m_native && m_native->hasFrame();
        if (frame != m_hasFrame) {
            m_hasFrame = frame;
            emit hasFrameChanged();
        }
    });
    pushViewport();
    if (m_active.load())
        m_native->setActive(true);
    qInfo() << "[kichi] 内嵌播放使用 Wayland 原生 surface";
}

void MpvVideoItem::setActive(bool active)
{
    if (m_active.load() == active)
        return;
    m_active.store(active);
    if (active) {
        pushViewport();
        m_poll.start();
        if (m_native)
            m_native->setActive(true);
    } else {
        m_poll.stop();
        if (m_native)
            m_native->setActive(false);
    }
    emit activeChanged();
}

void MpvVideoItem::poll()
{
    if (!m_active.load() || !m_native)
        return;
    m_native->poll();
    if (m_native->error() != m_error)
        setError(m_native->error());
    const QString colorInfo = m_native->colorInfo();
    if (colorInfo != m_colorInfo) {
        m_colorInfo = colorInfo;
        emit colorInfoChanged();
    }
}

void MpvVideoItem::pushViewport()
{
    QQuickWindow *win = window();
    if (!win)
        return;
    const int width = qRound(this->width());
    const int height = qRound(this->height());
    if (width <= 0 || height <= 0)
        return;
    const qreal dpr = win->effectiveDevicePixelRatio();
    kichi_embed_set_viewport(width, height, dpr);
    if (m_native)
        m_native->setGeometry(QSize(width, height), dpr);
}

void MpvVideoItem::geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry)
{
    QQuickItem::geometryChange(newGeometry, oldGeometry);
    pushViewport();
}

void MpvVideoItem::itemChange(ItemChange change, const ItemChangeData &value)
{
    if (change == ItemSceneChange && !value.window) {
        m_poll.stop();
        if (m_native) {
            m_native->setFrameCallback({});
            m_native.reset();
        }
        if (m_hasFrame) {
            m_hasFrame = false;
            emit hasFrameChanged();
        }
    }
    QQuickItem::itemChange(change, value);
}

void MpvVideoItem::setError(const QString &error)
{
    if (m_error == error)
        return;
    m_error = error;
    if (!error.isEmpty())
        qWarning().noquote() << "[kichi] 内嵌播放：" << error;
    emit errorChanged();
}
