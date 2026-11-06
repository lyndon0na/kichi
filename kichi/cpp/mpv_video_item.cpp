#include "mpv_video_item.h"

#include "embed_ffi.h"

#include <EGL/egl.h>
#include <EGL/eglext.h>

#include <QtCore/QDebug>
#include <QtGui/QImage>
#include <QtGui/QOpenGLContext>
#include <QtGui/qopenglcontext_platform.h>
#include <QtQuick/QQuickWindow>
#include <QtQuick/qsgtexture_platform.h>

#include <cmath>

namespace {

typedef EGLImageKHR (*CreateImageFn)(EGLDisplay, EGLContext, EGLenum, EGLClientBuffer,
                                     const EGLint *);
typedef EGLBoolean (*DestroyImageFn)(EGLDisplay, EGLImageKHR);
typedef void (*ImageTargetTextureFn)(unsigned int, void *);

CreateImageFn createImageFn()
{
    static auto fn = reinterpret_cast<CreateImageFn>(eglGetProcAddress("eglCreateImageKHR"));
    return fn;
}

DestroyImageFn destroyImageFn()
{
    static auto fn = reinterpret_cast<DestroyImageFn>(eglGetProcAddress("eglDestroyImageKHR"));
    return fn;
}

ImageTargetTextureFn imageTargetFn()
{
    static auto fn = reinterpret_cast<ImageTargetTextureFn>(
        eglGetProcAddress("glEGLImageTargetTexture2DOES"));
    return fn;
}

constexpr GLenum GLTexture2D = 0x0DE1;
constexpr GLint GLLinear = 0x2601;
constexpr GLint GLClampToEdge = 0x812F;
constexpr GLint GLTextureMinFilter = 0x2801;
constexpr GLint GLTextureMagFilter = 0x2800;
constexpr GLint GLTextureWrapS = 0x2802;
constexpr GLint GLTextureWrapT = 0x2803;

} // namespace

MpvVideoItem::MpvVideoItem(QQuickItem *parent)
    : QQuickItem(parent)
{
    setFlag(ItemHasContents, true);
    m_poll.setInterval(10);
    m_poll.setTimerType(Qt::PreciseTimer);
    connect(&m_poll, &QTimer::timeout, this, &MpvVideoItem::poll);
    connect(this, &QQuickItem::windowChanged, this, [this](QQuickWindow *window) {
        if (window) {
            // 上下文丢失后 Qt 会销毁场景节点；清掉纹理 / EGLImage，下一帧重建。
            // 占位纹理也是场景图资源，必须在这里（渲染线程）一并释放。
            connect(window, &QQuickWindow::sceneGraphInvalidated, this,
                    &MpvVideoItem::resetSceneGraph,
                    static_cast<Qt::ConnectionType>(Qt::DirectConnection
                                                    | Qt::UniqueConnection));
            pushViewport();
        }
    });
}

MpvVideoItem::~MpvVideoItem()
{
    {
        std::lock_guard<std::mutex> lock(m_frameMutex);
        if (m_frame) {
            kichi_embed_frame_free(m_frame);
            m_frame = nullptr;
        }
    }
    // 析构可能发生在没有 GL 上下文的时候：有上下文才删场景图对象，否则留给进程退出。
    // 节点不持有纹理（setOwnsTexture(false)），这里删的不会是节点正在用的指针。
    if (QOpenGLContext::currentContext())
        resetSceneGraph();
}

void MpvVideoItem::setActive(bool active)
{
    if (m_active.load() == active)
        return;
    m_active.store(active);
    if (active) {
        pushViewport();
        m_poll.start();
        update();
    } else {
        m_poll.stop();
        m_paintStale.store(true);
        // 旧 dmabuf 帧不在这里还：当前 GL 纹理可能还在采样它。交给渲染线程的
        // stale 分支，先把节点换成占位纹理、删旧纹理，再释放帧。即使这一帧已经
        // 不可见，下一次 sync 也会先走 stale 分支，不会拿旧纹理去画。
        update();
    }
    emit activeChanged();
}

void MpvVideoItem::poll()
{
    if (!m_active.load())
        return;
    pushViewport();
    if (window() && kichi_embed_frame_pending())
        update();
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
    kichi_embed_set_viewport(width, height, win->effectiveDevicePixelRatio());
}

void MpvVideoItem::geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry)
{
    QQuickItem::geometryChange(newGeometry, oldGeometry);
    pushViewport();
}

void MpvVideoItem::itemChange(ItemChange change, const ItemChangeData &value)
{
    if (change == ItemSceneChange && !value.window) {
        std::lock_guard<std::mutex> lock(m_frameMutex);
        if (m_frame) {
            kichi_embed_frame_free(m_frame);
            m_frame = nullptr;
        }
    }
    QQuickItem::itemChange(change, value);
}

QSGNode *MpvVideoItem::updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *)
{
    // Qt 拥有节点生命周期：旧节点被销毁后 oldNode 会是 nullptr，不能自己留指针。
    auto *node = static_cast<QSGSimpleTextureNode *>(oldNode);
    if (!node)
        node = new QSGSimpleTextureNode;
    node->setOwnsTexture(false);

    // QSGSimpleTextureNode::setTexture(nullptr) 在 Release 构建里直接解引用空指针
    // （Q_ASSERT 被编译掉）。场景图节点必须始终挂着有效纹理：没有画面时用 1×1
    // 黑色占位，顺便消掉 “No QSGTexture provided from updateSampledImage()” 刷屏。
    QSGTexture *placeholder = ensurePlaceholder();
    if (!placeholder) {
        // 没有窗口就建不了纹理；与其留一个空纹理节点，不如把节点从场景图摘掉。
        delete node;
        return nullptr;
    }

    // 停用后旧纹理可能指着已经还给 mpv 的 dmabuf。顺序必须是：
    // 节点先改挂占位纹理 → 再删旧纹理 / 释放旧帧，绝不能反过来。
    if (m_paintStale.exchange(false)) {
        node->setTexture(placeholder);
        node->setRect(boundingRect());
        node->setFiltering(QSGTexture::Nearest);
        resetGl();
        std::lock_guard<std::mutex> lock(m_frameMutex);
        if (m_frame) {
            kichi_embed_frame_free(m_frame);
            m_frame = nullptr;
        }
    }

    if (m_active.load()) {
        KichiFrame *frame = kichi_embed_frame_take();
        if (frame) {
            const bool shm = kichi_embed_frame_is_shm(frame);
            const bool ok = shm ? uploadShm(frame) : uploadDmabuf(frame);
            if (ok) {
                kichi_embed_frame_displayed(frame);
                if (shm) {
                    // 「软件帧」在 uploadShm 里已经复制成 Qt 纹理
                    kichi_embed_frame_free(frame);
                } else {
                    // dmabuf 纹理还指着这块内存：留到下一帧再还
                    std::lock_guard<std::mutex> lock(m_frameMutex);
                    if (m_frame)
                        kichi_embed_frame_free(m_frame);
                    m_frame = frame;
                }
            } else {
                kichi_embed_frame_free(frame);
            }
        }
    }

    // 任何时候都绑定一个有效纹理；还没有真实帧时就是全黑占位。
    QSGTexture *texture = m_texture ? m_texture : placeholder;
    QRectF target = boundingRect();
    if (m_texture && window()) {
        // 按帧的比例居中绘制，剩下的区域留给 QML 的黑色底：
        // mpv 的窗口尺寸跟上 item 之前，帧的宽高比可能还是旧的。
        double fitted[4] = {0.0, 0.0, 0.0, 0.0};
        if (kichi_embed_fit_rect(target.width(), target.height(),
                                 m_textureSize.width(), m_textureSize.height(), fitted)) {
            target = QRectF(fitted[0], fitted[1], fitted[2], fitted[3]);
        }
    }
    node->setTexture(texture);
    node->setRect(target);
    node->setFiltering(m_texture ? QSGTexture::Linear : QSGTexture::Nearest);
    // Qt 把 v=0 映射到矩形顶边，而 dmabuf / wl_shm 的第 0 行都是画面顶部：
    // 这里必须保持 NoTransform（镜像会上下倒置）。
    node->setTextureCoordinatesTransform(QSGSimpleTextureNode::NoTransform);
    return node;
}

QSGTexture *MpvVideoItem::ensurePlaceholder()
{
    if (m_placeholder)
        return m_placeholder;
    QQuickWindow *win = window();
    if (!win)
        return nullptr;
    QImage black(1, 1, QImage::Format_ARGB32);
    black.fill(Qt::black);
    m_placeholder = win->createTextureFromImage(black, QQuickWindow::TextureIsOpaque);
    if (m_placeholder)
        m_placeholder->setFiltering(QSGTexture::Nearest);
    return m_placeholder;
}

GLuint MpvVideoItem::ensureGlTexture(const QSize &size)
{
    QOpenGLContext *ctx = QOpenGLContext::currentContext();
    if (!ctx)
        return 0;
    QOpenGLFunctions *fn = ctx->functions();
    if (!m_glTexture) {
        fn->glGenTextures(1, &m_glTexture);
        if (!m_glTexture)
            return 0;
    }
    if (m_textureSize != size) {
        // 尺寸变了，旧的 QSGTexture 也要按新尺寸重建
        delete m_texture;
        m_texture = nullptr;
        m_nativeTexture = false;
        m_textureSize = size;
    }
    fn->glBindTexture(GLTexture2D, m_glTexture);
    fn->glTexParameteri(GLTexture2D, GLTextureMinFilter, GLLinear);
    fn->glTexParameteri(GLTexture2D, GLTextureMagFilter, GLLinear);
    fn->glTexParameteri(GLTexture2D, GLTextureWrapS, GLClampToEdge);
    fn->glTexParameteri(GLTexture2D, GLTextureWrapT, GLClampToEdge);
    return m_glTexture;
}

bool MpvVideoItem::uploadDmabuf(KichiFrame *frame)
{
    QQuickWindow *win = window();
    QOpenGLContext *ctx = QOpenGLContext::currentContext();
    if (!win || !ctx) {
        setError(QStringLiteral("内嵌播放：当前没有可用的 OpenGL 上下文"));
        return false;
    }
    auto *egl = ctx->nativeInterface<QNativeInterface::QEGLContext>();
    if (!egl || !egl->display()) {
        setError(QStringLiteral("内嵌播放需要 EGL/OpenGL 渲染后端"));
        return false;
    }
    m_eglDisplay = egl->display();
    const CreateImageFn createImage = createImageFn();
    const ImageTargetTextureFn target = imageTargetFn();
    if (!createImage || !target) {
        setError(QStringLiteral("内嵌播放：EGL 缺少 dmabuf 扩展"));
        return false;
    }

    const int width = static_cast<int>(kichi_embed_frame_width(frame));
    const int height = static_cast<int>(kichi_embed_frame_height(frame));
    const int fd = kichi_embed_frame_plane_fd(frame, 0);
    if (width <= 0 || height <= 0 || fd < 0) {
        setError(QStringLiteral("内嵌播放：收到的 dmabuf 帧无效"));
        return false;
    }
    GLuint texture = ensureGlTexture(QSize(width, height));
    if (!texture) {
        setError(QStringLiteral("内嵌播放：创建 GL 纹理失败"));
        return false;
    }

    const uint64_t modifier = kichi_embed_frame_modifier(frame);
    const EGLint attribs[] = {
        EGL_WIDTH,
        width,
        EGL_HEIGHT,
        height,
        EGL_LINUX_DRM_FOURCC_EXT,
        static_cast<EGLint>(kichi_embed_frame_fourcc(frame)),
        EGL_DMA_BUF_PLANE0_FD_EXT,
        fd,
        EGL_DMA_BUF_PLANE0_OFFSET_EXT,
        static_cast<EGLint>(kichi_embed_frame_plane_offset(frame, 0)),
        EGL_DMA_BUF_PLANE0_PITCH_EXT,
        static_cast<EGLint>(kichi_embed_frame_plane_stride(frame, 0)),
        EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT,
        static_cast<EGLint>(modifier & 0xFFFFFFFFu),
        EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT,
        static_cast<EGLint>(modifier >> 32),
        EGL_NONE,
    };
    EGLImageKHR image = createImage(m_eglDisplay, EGL_NO_CONTEXT, EGL_LINUX_DMA_BUF_EXT,
                                    nullptr, attribs);
    if (image == EGL_NO_IMAGE_KHR) {
        setError(QStringLiteral("内嵌播放：dmabuf 导入失败（显卡/驱动不支持线性格式）"));
        return false;
    }

    ctx->functions()->glBindTexture(GLTexture2D, texture);
    target(GLTexture2D, reinterpret_cast<void *>(image));
    if (m_eglImage) {
        if (const DestroyImageFn destroyImage = destroyImageFn())
            destroyImage(m_eglDisplay, m_eglImage);
    }
    m_eglImage = image;

    if (!m_nativeTexture || !m_texture) {
        delete m_texture;
        m_texture = QNativeInterface::QSGOpenGLTexture::fromNative(
            texture, win, QSize(width, height), QQuickWindow::TextureIsOpaque);
        if (!m_texture) {
            m_nativeTexture = false;
            setError(QStringLiteral("内嵌播放：原生纹理注册失败"));
            return false;
        }
        m_texture->setFiltering(QSGTexture::Linear);
        m_nativeTexture = true;
    }
    m_textureSize = QSize(width, height);
    return true;
}

bool MpvVideoItem::uploadShm(KichiFrame *frame)
{
    QQuickWindow *win = window();
    const int width = static_cast<int>(kichi_embed_frame_width(frame));
    const int height = static_cast<int>(kichi_embed_frame_height(frame));
    const uchar *pixels = kichi_embed_frame_pixels(frame);
    if (!win || !pixels || width <= 0 || height <= 0) {
        setError(QStringLiteral("内嵌播放：软件帧数据无效"));
        return false;
    }
    // qreal 只是借用像素：createTextureFromImage 会同步上传到 GPU
    const QImage image(pixels, width, height, width * 4, QImage::Format_ARGB32);
    QSGTexture *texture =
        win->createTextureFromImage(image, QQuickWindow::TextureIsOpaque);
    if (!texture) {
        setError(QStringLiteral("内嵌播放：上传软件帧失败"));
        return false;
    }
    delete m_texture;
    m_texture = texture;
    m_texture->setFiltering(QSGTexture::Linear);
    m_textureSize = QSize(width, height);
    return true;
}

void MpvVideoItem::resetSceneGraph()
{
    // 场景图失效 / 项析构：先清视频纹理，再清占位纹理。
    // 节点已经脱离场景图，不会再有指针指向它们。
    resetGl();
    delete m_placeholder;
    m_placeholder = nullptr;
}

void MpvVideoItem::resetGl()
{
    delete m_texture;
    m_texture = nullptr;
    if (m_eglImage && m_eglDisplay) {
        if (const DestroyImageFn destroyImage = destroyImageFn())
            destroyImage(m_eglDisplay, m_eglImage);
    }
    m_eglImage = nullptr;
    if (m_glTexture) {
        if (QOpenGLContext *ctx = QOpenGLContext::currentContext())
            ctx->functions()->glDeleteTextures(1, &m_glTexture);
        m_glTexture = 0;
    }
    m_textureSize = QSize();
}

void MpvVideoItem::setError(const QString &error)
{
    if (m_error == error)
        return;
    m_error = error;
    qWarning().noquote() << error;
    emit errorChanged();
}
