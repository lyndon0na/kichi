#pragma once

#include <atomic>
#include <memory>

#include <QtCore/QString>
#include <QtCore/QTimer>
#include <QtQml/qqmlregistration.h>
#include <QtQuick/QQuickItem>

class WaylandVideoSurface;

// 内嵌 libmpv 的画面项。
//
// 视频帧由 C++ 直接提交到 Qt 窗口下面的 wl_subsurface，不经过 Qt 场景图；
// 10bit / HDR / 硬解 dmabuf / tiled modifier 原样交给合成器。
class MpvVideoItem : public QQuickItem
{
    Q_OBJECT
    QML_NAMED_ELEMENT(MpvVideo)
    Q_PROPERTY(bool active READ active WRITE setActive NOTIFY activeChanged)
    Q_PROPERTY(QString error READ error NOTIFY errorChanged)
    Q_PROPERTY(bool hasFrame READ hasFrame NOTIFY hasFrameChanged)

public:
    explicit MpvVideoItem(QQuickItem *parent = nullptr);
    ~MpvVideoItem() override;

    bool active() const { return m_active.load(); }
    void setActive(bool active);
    QString error() const { return m_error; }
    bool hasFrame() const { return m_hasFrame; }

signals:
    void activeChanged();
    void errorChanged();
    void hasFrameChanged();

protected:
    void geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry) override;
    void itemChange(ItemChange change, const ItemChangeData &value) override;

private:
    void setupNative();
    void pushViewport();
    void poll();
    void setError(const QString &error);

    QTimer m_poll;
    std::atomic<bool> m_active{false};
    QString m_error;
    std::unique_ptr<WaylandVideoSurface> m_native;
    bool m_hasFrame = false;
};
