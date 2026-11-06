#pragma once

#include <atomic>
#include <mutex>

#include <QtCore/QSize>
#include <QtCore/QString>
#include <QtCore/QTimer>
#include <QtGui/QOpenGLFunctions>
#include <QtQml/qqmlregistration.h>
#include <QtQuick/QQuickItem>
#include <QtQuick/QSGSimpleTextureNode>
#include <QtQuick/QSGTexture>

struct KichiFrame;
typedef void *EGLDisplay;
typedef void *EGLImageKHR;

// 内嵌 libmpv 的画面项：从 Rust 侧取 dmabuf / wl_shm 帧，导入成 Qt 纹理。
// 只能在 Qt Quick 的 OpenGL RHI 后端下工作。
class MpvVideoItem : public QQuickItem
{
    Q_OBJECT
    QML_NAMED_ELEMENT(MpvVideo)
    Q_PROPERTY(bool active READ active WRITE setActive NOTIFY activeChanged)
    Q_PROPERTY(QString error READ error NOTIFY errorChanged)

public:
    explicit MpvVideoItem(QQuickItem *parent = nullptr);
    ~MpvVideoItem() override;

    bool active() const { return m_active.load(); }
    void setActive(bool active);
    QString error() const { return m_error; }

signals:
    void activeChanged();
    void errorChanged();

protected:
    QSGNode *updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *data) override;
    void geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry) override;
    void itemChange(ItemChange change, const ItemChangeData &value) override;

private:
    void pushViewport();
    void poll();
    bool uploadDmabuf(KichiFrame *frame);
    bool uploadShm(KichiFrame *frame);
    void resetGl();
    void resetSceneGraph();
    QSGTexture *ensurePlaceholder();
    void setError(const QString &error);
    GLuint ensureGlTexture(const QSize &size);

    QTimer m_poll;
    KichiFrame *m_frame = nullptr;
    // 场景图纹理只在渲染线程创建 / 删除；m_frame 的 GUI 侧释放要拿锁。
    std::mutex m_frameMutex;
    QSGTexture *m_texture = nullptr;
    // 1×1 黑色占位纹理：场景图节点任何时候都必须挂着有效纹理。
    QSGTexture *m_placeholder = nullptr;
    GLuint m_glTexture = 0;
    QSize m_textureSize;
    EGLDisplay m_eglDisplay = nullptr;
    EGLImageKHR m_eglImage = nullptr;
    bool m_nativeTexture = false;
    std::atomic<bool> m_active{false};
    // 停用后旧帧不能再画：新会话出第一帧前保持空白，而不是闪一下上一个视频。
    std::atomic<bool> m_paintStale{false};
    QString m_error;
};
