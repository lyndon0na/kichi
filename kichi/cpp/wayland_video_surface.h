#pragma once

#include <QPointer>
#include <QSize>
#include <QString>
#include <QtGlobal>

#include <cstdint>
#include <functional>
#include <unordered_map>

class QQuickWindow;
class QScreen;

struct wl_buffer;
struct KichiColorInfo;
struct KichiColorTarget;
struct wl_compositor;
struct wl_display;
struct wl_event_queue;
struct wl_registry;
struct wl_shm;
struct wl_subcompositor;
struct wl_subsurface;
struct wl_surface;
struct wp_color_management_surface_feedback_v1;
struct wp_color_management_surface_v1;
struct wp_color_manager_v1;
struct wp_content_type_manager_v1;
struct wp_content_type_v1;
struct wp_image_description_v1;
struct wp_presentation;
struct wp_presentation_feedback;
struct wp_viewport;
struct wp_viewporter;
struct zwp_linux_dmabuf_v1;
struct KichiFrame;

// 把 Rust 代理截获的 dmabuf / shm 帧直接提交到 Qt 窗口下面的 wl_subsurface。
//
// 视频像素不再经过 Qt 场景图：Qt 只负责窗口和控件，KWin 直接合成视频 surface。
// 10bit / HDR / 硬解 dmabuf / tiled modifier 都能原样交给合成器；色彩跟随
// 播放窗口所在 output 的 preferred image description，并按 mpv 的 target
// 模式归一化成 PQ / 相对亮度描述。
class WaylandVideoSurface
{
public:
    using FrameCallback = std::function<void()>;

    explicit WaylandVideoSurface(QQuickWindow *window);
    ~WaylandVideoSurface();

    bool available() const { return m_available; }
    QString error() const { return m_error; }
    bool hasFrame() const { return m_hasFrame; }
    /// 当前视频 surface 使用的显示目标（调试展示用）。
    QString colorInfo() const { return m_colorInfo; }

    void setActive(bool active);
    void setGeometry(const QSize &logicalSize, qreal devicePixelRatio);
    void poll();

    void setFrameCallback(FrameCallback callback) { m_onFrame = std::move(callback); }

    // 供 C 语言 Wayland listener 调用（不是给业务层用的 API）。
    void applyImageDescription(wp_image_description_v1 *description, bool allowInfo);
    bool rememberPreferred(const KichiColorInfo &info);
    void onBufferReleased(wl_buffer *buffer);
    void onPresentationPresented(struct wp_presentation_feedback *feedback, uint32_t tvSecHi,
                                 uint32_t tvSecLo, uint32_t tvNsec, uint32_t refresh,
                                 uint32_t seqHi, uint32_t seqLo, uint32_t flags);
    void onPresentationDiscarded(struct wp_presentation_feedback *feedback);
    void noteColorFeature(uint32_t feature);
    void noteTransfer(uint32_t transfer);
    void bindGlobal(wl_registry *registry, uint32_t name, const char *interface, uint32_t version);
    void requestPreferredUpdate() { m_preferredDirty = true; }

private:
    bool initWayland();
    void queryPreferredTarget();
    void queryPlaybackPreferred();
    void publishTarget();
    bool ensureChild();
    void teardownChild();
    void pollFrames();
    void commitFrame(KichiFrame *frame);
    bool createDmabufBuffer(KichiFrame *frame, int width, int height, wl_buffer **out);
    bool createShmBuffer(KichiFrame *frame, int width, int height, wl_buffer **out);
    void dispatch();
    void applyColorDescription();
    void applyColorInfo(const KichiColorInfo &preferred, const KichiColorTarget &target);
    bool applyParametric(const KichiColorTarget &target);
    void notifyFrame();
    void setError(const QString &error);

    QPointer<QQuickWindow> m_window;
    QString m_error;
    bool m_available = false;
    bool m_active = false;
    bool m_hasFrame = false;
    QSize m_logicalSize;
    FrameCallback m_onFrame;

    wl_display *m_display = nullptr;
    wl_event_queue *m_queue = nullptr;
    wl_registry *m_registry = nullptr;
    wl_compositor *m_compositor = nullptr;
    wl_subcompositor *m_subcompositor = nullptr;
    wl_shm *m_shm = nullptr;
    zwp_linux_dmabuf_v1 *m_dmabuf = nullptr;
    wp_viewporter *m_viewporter = nullptr;
    wp_color_manager_v1 *m_colorManager = nullptr;
    wp_presentation *m_presentation = nullptr;
    wp_content_type_manager_v1 *m_contentTypeManager = nullptr;

    wl_surface *m_parent = nullptr;
    wl_surface *m_child = nullptr;
    wl_subsurface *m_subsurface = nullptr;
    wp_viewport *m_viewport = nullptr;
    wp_color_management_surface_v1 *m_colorSurface = nullptr;
    wp_color_management_surface_feedback_v1 *m_childFeedback = nullptr;
    wp_color_management_surface_feedback_v1 *m_parentFeedback = nullptr;
    wp_content_type_v1 *m_contentType = nullptr;
    /// 真实 child surface 上还没兑现的 presentation feedback。
    std::unordered_map<struct wp_presentation_feedback *, uint64_t> m_pendingFeedback;
    QScreen *m_screen = nullptr;
    quintptr m_parentId = 0;
    bool m_preferredDirty = false;
    QString m_colorInfo;

    bool m_cmParametric = false;
    bool m_cmSetPrimaries = false;
    bool m_cmSetLuminances = false;
    bool m_cmHasPq = false;
    bool m_cmHasGamma22 = false;
    bool m_cmHasSrgb = false;
    bool m_cmHasHlg = false;

    std::unordered_map<wl_buffer *, KichiFrame *> m_frames;
};
