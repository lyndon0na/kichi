#include "wayland_video_surface.h"

#include "embed_ffi.h"

#include <QDebug>
#include <QGuiApplication>
#include <QQuickWindow>
#include <QScreen>
#include <QtGui/qguiapplication_platform.h>

#include <wayland-client.h>

#include <fcntl.h>
#include <sys/mman.h>
#include <unistd.h>

#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstring>
#include <deque>
#include <mutex>

#include "protocol/color-management-v1-client-protocol.h"
#include "protocol/content-type-v1-client-protocol.h"
#include "protocol/linux-dmabuf-v1-client-protocol.h"
#include "protocol/presentation-time-client-protocol.h"
#include "protocol/viewporter-client-protocol.h"

namespace {

std::mutex gTargetMutex;
KichiColorTarget gPreferredTarget{};
bool gPreferredTargetValid = false;

// 真实 wp_presentation 事件队列：C++ 回调（Qt 线程）写入，代理线程取走。
std::mutex gPresentationMutex;
std::deque<KichiPresentationEvent> gPresentationQueue;
std::atomic<uint32_t> gPresentationClock{1};

void pushPresentationEvent(const KichiPresentationEvent &event)
{
    std::lock_guard<std::mutex> lock(gPresentationMutex);
    gPresentationQueue.push_back(event);
}

int mapPrimaries(uint32_t primaries)
{
    switch (primaries) {
    case WP_COLOR_MANAGER_V1_PRIMARIES_SRGB:
        return KICHI_PRIMARIES_BT709;
    case WP_COLOR_MANAGER_V1_PRIMARIES_BT2020:
        return KICHI_PRIMARIES_BT2020;
    case WP_COLOR_MANAGER_V1_PRIMARIES_DISPLAY_P3:
        return KICHI_PRIMARIES_DISPLAY_P3;
    default:
        return KICHI_PRIMARIES_UNKNOWN;
    }
}

int mapTransfer(uint32_t transfer)
{
    switch (transfer) {
    case WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_GAMMA22:
        return KICHI_TRANSFER_GAMMA22;
    case WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_SRGB:
        return KICHI_TRANSFER_SRGB;
    case WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_ST2084_PQ:
        return KICHI_TRANSFER_PQ;
    case WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_HLG:
        return KICHI_TRANSFER_HLG;
    default:
        return KICHI_TRANSFER_UNKNOWN;
    }
}

uint32_t waylandPrimaries(int primaries)
{
    switch (primaries) {
    case KICHI_PRIMARIES_BT2020:
        return WP_COLOR_MANAGER_V1_PRIMARIES_BT2020;
    case KICHI_PRIMARIES_DISPLAY_P3:
        return WP_COLOR_MANAGER_V1_PRIMARIES_DISPLAY_P3;
    case KICHI_PRIMARIES_BT709:
    default:
        return WP_COLOR_MANAGER_V1_PRIMARIES_SRGB;
    }
}

uint32_t waylandTransfer(int transfer)
{
    switch (transfer) {
    case KICHI_TRANSFER_PQ:
        return WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_ST2084_PQ;
    case KICHI_TRANSFER_HLG:
        return WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_HLG;
    case KICHI_TRANSFER_SRGB:
        return WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_SRGB;
    case KICHI_TRANSFER_GAMMA22:
    default:
        return WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_GAMMA22;
    }
}

struct DescState {
    WaylandVideoSurface *self;
};

struct InfoState {
    WaylandVideoSurface *self;
    uint32_t primaries = 0;
    uint32_t transfer = 0;
    uint32_t minLuminance = 0;
    uint32_t maxLuminance = 0;
    uint32_t referenceLuminance = 0;
    bool hasLuminance = false;
};

struct TargetQuery {
    bool ready = false;
    bool failed = false;
    uint32_t primaries = 0;
    uint32_t transfer = 0;
    uint32_t minLuminance = 0;
    uint32_t maxLuminance = 0;
    uint32_t referenceLuminance = 0;
    bool hasLuminance = false;
};

void registryGlobal(void *data, wl_registry *registry, uint32_t name, const char *interface,
                    uint32_t version)
{
    static_cast<WaylandVideoSurface *>(data)->bindGlobal(registry, name, interface, version);
}

void registryGlobalRemove(void *, wl_registry *, uint32_t)
{
}

const wl_registry_listener registryListener = {
    registryGlobal,
    registryGlobalRemove,
};

void bufferRelease(void *data, wl_buffer *buffer)
{
    static_cast<WaylandVideoSurface *>(data)->onBufferReleased(buffer);
}

const wl_buffer_listener bufferListener = {
    bufferRelease,
};

void presentationClockId(void *, wp_presentation *, uint32_t clockId)
{
    gPresentationClock.store(clockId);
}

const wp_presentation_listener presentationListener = {
    presentationClockId,
};

void feedbackSyncOutput(void *, struct wp_presentation_feedback *, wl_output *)
{
}

void feedbackPresented(void *data, struct wp_presentation_feedback *feedback, uint32_t tvSecHi,
                       uint32_t tvSecLo, uint32_t tvNsec, uint32_t refresh, uint32_t seqHi,
                       uint32_t seqLo, uint32_t flags)
{
    static_cast<WaylandVideoSurface *>(data)->onPresentationPresented(
        feedback, tvSecHi, tvSecLo, tvNsec, refresh, seqHi, seqLo, flags);
}

void feedbackDiscarded(void *data, struct wp_presentation_feedback *feedback)
{
    static_cast<WaylandVideoSurface *>(data)->onPresentationDiscarded(feedback);
}

const wp_presentation_feedback_listener presentationFeedbackListener = {
    feedbackSyncOutput,
    feedbackPresented,
    feedbackDiscarded,
};

// 播放窗口换到另一个 output / 用户改 HDR 设置时 KWin 会更新 preferred。
// 回调里只打标记，真正查询放到下一次 poll，避免在 dispatch 里嵌套 roundtrip。
void feedbackPreferredChanged(void *data, wp_color_management_surface_feedback_v1 *, uint32_t)
{
    static_cast<WaylandVideoSurface *>(data)->requestPreferredUpdate();
}

void feedbackPreferredChanged2(void *data, wp_color_management_surface_feedback_v1 *, uint32_t,
                               uint32_t)
{
    static_cast<WaylandVideoSurface *>(data)->requestPreferredUpdate();
}

const wp_color_management_surface_feedback_v1_listener feedbackListener = {
    feedbackPreferredChanged,
    feedbackPreferredChanged2,
};

void cmSupportedIntent(void *, wp_color_manager_v1 *, uint32_t)
{
}

void cmSupportedFeature(void *data, wp_color_manager_v1 *, uint32_t feature)
{
    static_cast<WaylandVideoSurface *>(data)->noteColorFeature(feature);
}

void cmSupportedTf(void *data, wp_color_manager_v1 *, uint32_t transfer)
{
    static_cast<WaylandVideoSurface *>(data)->noteTransfer(transfer);
}

void cmSupportedPrimaries(void *, wp_color_manager_v1 *, uint32_t)
{
}

void cmDone(void *, wp_color_manager_v1 *)
{
}

const wp_color_manager_v1_listener colorManagerListener = {
    cmSupportedIntent,
    cmSupportedFeature,
    cmSupportedTf,
    cmSupportedPrimaries,
    cmDone,
};


void infoDone(void *data, wp_image_description_info_v1 *info)
{
    // done 是最后一条事件；info 对象本身要显式销毁（description 销毁不会带走它）。
    if (info)
        wp_image_description_info_v1_destroy(info);
    auto *state = static_cast<InfoState *>(data);
    qInfo() << "[kichi] 内嵌 Wayland 色彩：preferred primaries=" << state->primaries
            << "transfer=" << state->transfer << "lum=" << state->minLuminance << "/"
            << state->maxLuminance << "/" << state->referenceLuminance;
    if (state->hasLuminance) {
        // 主窗口 / 播放窗口的 preferred 才是 SDR target；child preferred 只在
        // 没有主窗口 target 时兜底（它经常是默认 sRGB/80，不能覆盖 HDR）。
        bool needFallback;
        {
            std::lock_guard<std::mutex> lock(gTargetMutex);
            needFallback = !gPreferredTargetValid;
        }
        if (needFallback) {
            state->self->rememberPreferred(state->primaries, state->transfer,
                                           state->minLuminance, state->maxLuminance,
                                           state->referenceLuminance);
        }
    }
    delete state;
}

void infoIcc(void *, wp_image_description_info_v1 *, int32_t fd, uint32_t)
{
    if (fd >= 0)
        close(fd);
}

void infoPrimaries(void *, wp_image_description_info_v1 *, int32_t, int32_t, int32_t, int32_t,
                   int32_t, int32_t, int32_t, int32_t)
{
}

void infoPrimariesNamed(void *data, wp_image_description_info_v1 *, uint32_t primaries)
{
    static_cast<InfoState *>(data)->primaries = primaries;
}

void infoTfPower(void *, wp_image_description_info_v1 *, uint32_t)
{
}

void infoTfNamed(void *data, wp_image_description_info_v1 *, uint32_t transfer)
{
    static_cast<InfoState *>(data)->transfer = transfer;
}

void infoLuminances(void *data, wp_image_description_info_v1 *, uint32_t minLuminance,
                    uint32_t maxLuminance, uint32_t referenceLuminance)
{
    auto *state = static_cast<InfoState *>(data);
    state->minLuminance = minLuminance;
    state->maxLuminance = maxLuminance;
    state->referenceLuminance = referenceLuminance;
    state->hasLuminance = true;
}

void infoTargetPrimaries(void *, wp_image_description_info_v1 *, int32_t, int32_t, int32_t,
                         int32_t, int32_t, int32_t, int32_t, int32_t)
{
}

void infoTargetLuminance(void *, wp_image_description_info_v1 *, uint32_t, uint32_t)
{
}

void infoTargetMaxCll(void *, wp_image_description_info_v1 *, uint32_t)
{
}

void infoTargetMaxFall(void *, wp_image_description_info_v1 *, uint32_t)
{
}

const wp_image_description_info_v1_listener infoListener = {
    infoDone,            infoIcc,             infoPrimaries,    infoPrimariesNamed,
    infoTfPower,         infoTfNamed,         infoLuminances,   infoTargetPrimaries,
    infoTargetLuminance, infoTargetMaxCll,    infoTargetMaxFall,
};

void descFailed(void *data, wp_image_description_v1 *description, uint32_t cause,
                const char *message)
{
    auto *state = static_cast<DescState *>(data);
    qWarning() << "[kichi] 内嵌 Wayland 色彩描述失败：" << cause << (message ? message : "");
    wp_image_description_v1_destroy(description);
    delete state;
}

void descReady(void *data, wp_image_description_v1 *description, uint32_t)
{
    auto *state = static_cast<DescState *>(data);
    state->self->applyImageDescription(description);
    delete state;
}

void descReady2(void *data, wp_image_description_v1 *description, uint32_t, uint32_t)
{
    descReady(data, description, 0);
}

const wp_image_description_v1_listener descListener = {
    descFailed,
    descReady,
    descReady2,
};

// 启动时先查主窗口 surface 的 preferred，给 mpv 一个初值；播放窗口映射后
// 再以播放窗口 target 为准（见 queryPlaybackPreferred）。
void targetDescFailed(void *data, wp_image_description_v1 *description, uint32_t, const char *)
{
    auto *query = static_cast<TargetQuery *>(data);
    query->failed = true;
    wp_image_description_v1_destroy(description);
}

void targetDescReady(void *data, wp_image_description_v1 *, uint32_t)
{
    static_cast<TargetQuery *>(data)->ready = true;
}

void targetDescReady2(void *data, wp_image_description_v1 *description, uint32_t, uint32_t)
{
    targetDescReady(data, description, 0);
}

const wp_image_description_v1_listener targetDescListener = {
    targetDescFailed,
    targetDescReady,
    targetDescReady2,
};

void targetInfoDone(void *, wp_image_description_info_v1 *info)
{
    if (info)
        wp_image_description_info_v1_destroy(info);
}

void targetInfoIcc(void *, wp_image_description_info_v1 *, int32_t fd, uint32_t)
{
    if (fd >= 0)
        close(fd);
}

void targetInfoPrimaries(void *, wp_image_description_info_v1 *, int32_t, int32_t, int32_t,
                         int32_t, int32_t, int32_t, int32_t, int32_t)
{
}

void targetInfoPrimariesNamed(void *data, wp_image_description_info_v1 *, uint32_t primaries)
{
    static_cast<TargetQuery *>(data)->primaries = primaries;
}

void targetInfoTfPower(void *, wp_image_description_info_v1 *, uint32_t)
{
}

void targetInfoTfNamed(void *data, wp_image_description_info_v1 *, uint32_t transfer)
{
    static_cast<TargetQuery *>(data)->transfer = transfer;
}

void targetInfoLuminances(void *data, wp_image_description_info_v1 *, uint32_t minLuminance,
                          uint32_t maxLuminance, uint32_t referenceLuminance)
{
    auto *query = static_cast<TargetQuery *>(data);
    query->minLuminance = minLuminance;
    query->maxLuminance = maxLuminance;
    query->referenceLuminance = referenceLuminance;
    query->hasLuminance = true;
}

void targetInfoTargetPrimaries(void *, wp_image_description_info_v1 *, int32_t, int32_t, int32_t,
                               int32_t, int32_t, int32_t, int32_t, int32_t)
{
}

void targetInfoTargetLuminance(void *, wp_image_description_info_v1 *, uint32_t, uint32_t)
{
}

void targetInfoTargetMaxCll(void *, wp_image_description_info_v1 *, uint32_t)
{
}

void targetInfoTargetMaxFall(void *, wp_image_description_info_v1 *, uint32_t)
{
}

const wp_image_description_info_v1_listener targetInfoListener = {
    targetInfoDone,          targetInfoIcc,          targetInfoPrimaries,
    targetInfoPrimariesNamed, targetInfoTfPower,      targetInfoTfNamed,
    targetInfoLuminances,    targetInfoTargetPrimaries, targetInfoTargetLuminance,
    targetInfoTargetMaxCll,  targetInfoTargetMaxFall,
};

} // namespace

// ---------------------------------------------------------------- FFI exports

extern "C" bool kichi_wayland_preferred_target(KichiColorTarget *out)
{
    if (!out)
        return false;
    std::lock_guard<std::mutex> lock(gTargetMutex);
    if (!gPreferredTargetValid)
        return false;
    *out = gPreferredTarget;
    return true;
}

extern "C" uint32_t kichi_embed_presentation_clock(void)
{
    return gPresentationClock.load();
}

extern "C" bool kichi_embed_presentation_take(KichiPresentationEvent *out)
{
    if (!out)
        return false;
    std::lock_guard<std::mutex> lock(gPresentationMutex);
    if (gPresentationQueue.empty())
        return false;
    *out = gPresentationQueue.front();
    gPresentationQueue.pop_front();
    return true;
}

// ------------------------------------------------------------- construction

WaylandVideoSurface::WaylandVideoSurface(QQuickWindow *window)
    : m_window(window)
{
    if (!initWayland()) {
        m_available = false;
        return;
    }
    m_available = true;
    queryPreferredTarget();
}

WaylandVideoSurface::~WaylandVideoSurface()
{
    teardownChild();
    if (m_colorManager)
        wp_color_manager_v1_destroy(m_colorManager);
    if (m_contentTypeManager)
        wp_content_type_manager_v1_destroy(m_contentTypeManager);
    if (m_presentation)
        wp_presentation_destroy(m_presentation);
    if (m_viewporter)
        wp_viewporter_destroy(m_viewporter);
    if (m_dmabuf)
        zwp_linux_dmabuf_v1_destroy(m_dmabuf);
    if (m_shm)
        wl_shm_destroy(m_shm);
    if (m_subcompositor)
        wl_subcompositor_destroy(m_subcompositor);
    if (m_compositor)
        wl_compositor_destroy(m_compositor);
    if (m_registry)
        wl_registry_destroy(m_registry);
    if (m_queue)
        wl_event_queue_destroy(m_queue);
}

// ------------------------------------------------------------------ Wayland

void WaylandVideoSurface::bindGlobal(wl_registry *registry, uint32_t name,
                                     const char *interface, uint32_t version)
{
    if (std::strcmp(interface, wl_compositor_interface.name) == 0) {
        m_compositor = static_cast<wl_compositor *>(
            wl_registry_bind(registry, name, &wl_compositor_interface,
                             std::min<uint32_t>(version, 4)));
        wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_compositor), m_queue);
    } else if (std::strcmp(interface, wl_subcompositor_interface.name) == 0) {
        m_subcompositor = static_cast<wl_subcompositor *>(
            wl_registry_bind(registry, name, &wl_subcompositor_interface, 1));
        wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_subcompositor), m_queue);
    } else if (std::strcmp(interface, wl_shm_interface.name) == 0) {
        m_shm = static_cast<wl_shm *>(wl_registry_bind(registry, name, &wl_shm_interface, 1));
        wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_shm), m_queue);
    } else if (std::strcmp(interface, zwp_linux_dmabuf_v1_interface.name) == 0) {
        m_dmabuf = static_cast<zwp_linux_dmabuf_v1 *>(
            wl_registry_bind(registry, name, &zwp_linux_dmabuf_v1_interface,
                             std::min<uint32_t>(version, 4)));
        wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_dmabuf), m_queue);
    } else if (std::strcmp(interface, wp_viewporter_interface.name) == 0) {
        m_viewporter = static_cast<wp_viewporter *>(
            wl_registry_bind(registry, name, &wp_viewporter_interface, 1));
        wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_viewporter), m_queue);
    } else if (std::strcmp(interface, wp_color_manager_v1_interface.name) == 0) {
        m_colorManager = static_cast<wp_color_manager_v1 *>(
            wl_registry_bind(registry, name, &wp_color_manager_v1_interface,
                             std::min<uint32_t>(version, 1)));
        wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_colorManager), m_queue);
        wp_color_manager_v1_add_listener(m_colorManager, &colorManagerListener, this);
    } else if (std::strcmp(interface, wp_presentation_interface.name) == 0) {
        m_presentation = static_cast<wp_presentation *>(
            wl_registry_bind(registry, name, &wp_presentation_interface,
                             std::min<uint32_t>(version, 2)));
        wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_presentation), m_queue);
        wp_presentation_add_listener(m_presentation, &presentationListener, this);
    } else if (std::strcmp(interface, wp_content_type_manager_v1_interface.name) == 0) {
        m_contentTypeManager = static_cast<wp_content_type_manager_v1 *>(
            wl_registry_bind(registry, name, &wp_content_type_manager_v1_interface, 1));
        wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_contentTypeManager), m_queue);
    }
}

bool WaylandVideoSurface::initWayland()
{
    auto *waylandApp = qGuiApp->nativeInterface<QNativeInterface::QWaylandApplication>();
    if (!waylandApp || !waylandApp->display()) {
        setError(QStringLiteral("当前会话没有 Wayland"));
        return false;
    }
    m_display = waylandApp->display();
    m_queue = wl_display_create_queue(m_display);
    if (!m_queue) {
        setError(QStringLiteral("创建 Wayland 事件队列失败"));
        return false;
    }
    m_registry = wl_display_get_registry(m_display);
    wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_registry), m_queue);
    wl_registry_add_listener(m_registry, &registryListener, this);
    (void)wl_display_roundtrip_queue(m_display, m_queue);

    if (!m_compositor || !m_subcompositor || !m_shm || !m_dmabuf) {
        setError(QStringLiteral("Wayland 合成器缺少 compositor / subcompositor / shm / dmabuf"));
        return false;
    }
    return true;
}

void WaylandVideoSurface::queryPreferredTarget()
{
    if (!m_colorManager)
        return;

    QWindow *mainWindow = nullptr;
    for (QWindow *window : qGuiApp->topLevelWindows()) {
        if (!window || window == m_window.data() || !window->isVisible())
            continue;
        if (window->winId() < 0x10000)
            continue;
        mainWindow = window;
        break;
    }

    TargetQuery query;
    bool queried = false;
    if (mainWindow) {
        auto *surface = reinterpret_cast<wl_surface *>(mainWindow->winId());
        auto *feedback = wp_color_manager_v1_get_surface_feedback(m_colorManager, surface);
        if (feedback) {
            wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(feedback), m_queue);
            auto *description = wp_color_management_surface_feedback_v1_get_preferred(feedback);
            if (description) {
                wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(description), m_queue);
                wp_image_description_v1_add_listener(description, &targetDescListener, &query);
                (void)wl_display_roundtrip_queue(m_display, m_queue);
                if (query.ready && !query.failed) {
                    auto *info = wp_image_description_v1_get_information(description);
                    if (info) {
                        wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(info), m_queue);
                        wp_image_description_info_v1_add_listener(info, &targetInfoListener,
                                                                  &query);
                        (void)wl_display_roundtrip_queue(m_display, m_queue);
                    }
                }
                wp_image_description_v1_destroy(description);
                queried = query.ready && query.hasLuminance;
            }
            wp_color_management_surface_feedback_v1_destroy(feedback);
        }
    }

    if (queried) {
        if (rememberPreferred(query.primaries, query.transfer, query.minLuminance,
                              query.maxLuminance, query.referenceLuminance)) {
            publishTarget();
        }
        qInfo() << "[kichi] 内嵌 Wayland 首选色彩：primaries=" << query.primaries
                << "transfer=" << query.transfer << "lum=" << query.minLuminance << "/"
                << query.maxLuminance << "/" << query.referenceLuminance;
    } else {
        // KWin 默认 SDR 混合空间；主窗口还不可查时用它兜底。
        KichiColorTarget fallback{};
        fallback.primaries = KICHI_PRIMARIES_BT709;
        fallback.transfer = KICHI_TRANSFER_SRGB;
        fallback.min_luminance_10000 = 2000;
        fallback.max_luminance = 80;
        fallback.reference_luminance = 80;
        {
            std::lock_guard<std::mutex> lock(gTargetMutex);
            gPreferredTarget = fallback;
            gPreferredTargetValid = true;
        }
        applyColorInfo(fallback);
        qInfo() << "[kichi] 内嵌 Wayland 首选色彩：回退 sRGB/BT709/80";
    }
}

// 播放窗口真正映射后，它所在的 output 才是视频 surface 的混合空间。
// 主窗口 target 只用于启动时 mpv 的初值；这里查到就以播放窗口为准。
void WaylandVideoSurface::queryPlaybackPreferred()
{
    if (!m_colorManager || !m_parentFeedback || !m_window)
        return;
    if (!m_window->isVisible() || !m_window->isExposed())
        return;

    TargetQuery query;
    auto *description = wp_color_management_surface_feedback_v1_get_preferred(m_parentFeedback);
    if (!description)
        return;
    wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(description), m_queue);
    wp_image_description_v1_add_listener(description, &targetDescListener, &query);
    (void)wl_display_roundtrip_queue(m_display, m_queue);
    if (query.ready && !query.failed) {
        auto *info = wp_image_description_v1_get_information(description);
        if (info) {
            wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(info), m_queue);
            wp_image_description_info_v1_add_listener(info, &targetInfoListener, &query);
            (void)wl_display_roundtrip_queue(m_display, m_queue);
        }
    }
    wp_image_description_v1_destroy(description);

    if (!query.ready || query.failed || !query.hasLuminance)
        return;
    if (!rememberPreferred(query.primaries, query.transfer, query.minLuminance,
                           query.maxLuminance, query.referenceLuminance))
        return;

    // 先让视频 surface 用新描述，再把同一个 target 投给 mpv 线程。
    applyColorDescription();
    publishTarget();
    qInfo() << "[kichi] 播放窗口首选色彩更新（"
            << (m_screen ? m_screen->name() : QStringLiteral("未知屏幕"))
            << "）：primaries=" << query.primaries << "transfer=" << query.transfer
            << "lum=" << query.minLuminance << "/" << query.maxLuminance << "/"
            << query.referenceLuminance;
}

void WaylandVideoSurface::noteColorFeature(uint32_t feature)
{
    switch (feature) {
    case WP_COLOR_MANAGER_V1_FEATURE_PARAMETRIC:
        m_cmParametric = true;
        break;
    case WP_COLOR_MANAGER_V1_FEATURE_SET_PRIMARIES:
        m_cmSetPrimaries = true;
        break;
    case WP_COLOR_MANAGER_V1_FEATURE_SET_LUMINANCES:
        m_cmSetLuminances = true;
        break;
    default:
        break;
    }
}

void WaylandVideoSurface::noteTransfer(uint32_t transfer)
{
    switch (transfer) {
    case WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_ST2084_PQ:
        m_cmHasPq = true;
        break;
    case WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_GAMMA22:
        m_cmHasGamma22 = true;
        break;
    case WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_SRGB:
        m_cmHasSrgb = true;
        break;
    case WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_HLG:
        m_cmHasHlg = true;
        break;
    default:
        break;
    }
}

// ------------------------------------------------------------------- surface

bool WaylandVideoSurface::ensureChild()
{
    if (!m_window || !m_window->isVisible())
        return false;

    const WId winId = m_window->winId();
    if (winId < 0x10000)
        return false;
    if (m_child && m_parentId == quintptr(winId))
        return true;

    teardownChild();
    m_parent = reinterpret_cast<wl_surface *>(winId);
    m_parentId = quintptr(winId);

    // 父 surface 必须不是整块 opaque，否则下面的视频永远看不到。
    wl_surface_set_opaque_region(m_parent, nullptr);

    // content-type 是 surface 级协议：mpv 的 surface 是假的，必须设在真正
    // 参与合成的播放窗口 surface 上。合成器据此知道这块内容以视频为主。
    if (m_contentTypeManager) {
        m_contentType = wp_content_type_manager_v1_get_surface_content_type(m_contentTypeManager,
                                                                            m_parent);
        if (m_contentType) {
            wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_contentType), m_queue);
            wp_content_type_v1_set_content_type(m_contentType, WP_CONTENT_TYPE_V1_TYPE_VIDEO);
        }
    }

    m_child = wl_compositor_create_surface(m_compositor);
    if (!m_child)
        return false;
    wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_child), m_queue);
    wl_surface_set_buffer_scale(m_child, 1);

    m_subsurface = wl_subcompositor_get_subsurface(m_subcompositor, m_child, m_parent);
    if (!m_subsurface)
        return false;
    wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_subsurface), m_queue);
    wl_subsurface_place_below(m_subsurface, m_parent);
    wl_subsurface_set_desync(m_subsurface);
    wl_subsurface_set_position(m_subsurface, 0, 0);

    if (m_viewporter) {
        m_viewport = wp_viewporter_get_viewport(m_viewporter, m_child);
        if (m_viewport) {
            wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_viewport), m_queue);
            const QSize size = m_logicalSize.isValid() ? m_logicalSize : m_window->size();
            wp_viewport_set_destination(m_viewport, size.width(), size.height());
        }
    }

    if (m_colorManager) {
        m_colorSurface = wp_color_manager_v1_get_surface(m_colorManager, m_child);
        if (m_colorSurface)
            wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_colorSurface), m_queue);
        m_childFeedback = wp_color_manager_v1_get_surface_feedback(m_colorManager, m_child);
        if (m_childFeedback)
            wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_childFeedback), m_queue);
        // 播放窗口自己的 feedback 才反映它所在 output；child feedback 在映射前
        // 常是默认 sRGB/80（见 docs/pitfalls.md 第 61 条）。
        m_parentFeedback = wp_color_manager_v1_get_surface_feedback(m_colorManager, m_parent);
        if (m_parentFeedback) {
            wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(m_parentFeedback), m_queue);
            wp_color_management_surface_feedback_v1_add_listener(m_parentFeedback,
                                                                 &feedbackListener, this);
        }
        applyColorDescription();
        // 窗口这时已映射；下一拍 poll 查播放窗口的实际 preferred。
        m_preferredDirty = true;
    }

    // 让 Qt 再提交一次父 surface，把 opaque_region=null 生效。
    m_window->update();
    wl_display_flush(m_display);
    return true;
}

void WaylandVideoSurface::teardownChild()
{
    for (auto &[buffer, frame] : m_frames) {
        if (frame)
            kichi_embed_frame_free(frame);
        wl_buffer_destroy(buffer);
    }
    m_frames.clear();

    for (auto &[feedback, token] : m_pendingFeedback) {
        // 不要在 surface 销毁时让 mpv 永远等这些 feedback：明确回 discarded。
        KichiPresentationEvent event{};
        event.token = token;
        event.presented = 0;
        pushPresentationEvent(event);
        wp_presentation_feedback_destroy(feedback);
    }
    m_pendingFeedback.clear();

    if (m_contentType) {
        wp_content_type_v1_destroy(m_contentType);
        m_contentType = nullptr;
    }
    if (m_colorSurface) {
        wp_color_management_surface_v1_destroy(m_colorSurface);
        m_colorSurface = nullptr;
    }
    if (m_childFeedback) {
        wp_color_management_surface_feedback_v1_destroy(m_childFeedback);
        m_childFeedback = nullptr;
    }
    if (m_parentFeedback) {
        wp_color_management_surface_feedback_v1_destroy(m_parentFeedback);
        m_parentFeedback = nullptr;
    }
    if (m_viewport) {
        wp_viewport_destroy(m_viewport);
        m_viewport = nullptr;
    }
    if (m_subsurface) {
        wl_subsurface_destroy(m_subsurface);
        m_subsurface = nullptr;
    }
    if (m_child) {
        wl_surface_destroy(m_child);
        m_child = nullptr;
    }
    m_parent = nullptr;
    m_parentId = 0;
    m_screen = nullptr;
    m_preferredDirty = false;
    m_hasFrame = false;
    notifyFrame();
}

void WaylandVideoSurface::setActive(bool active)
{
    if (m_active == active)
        return;
    m_active = active;
    if (!active) {
        teardownChild();
        return;
    }
    if (!m_logicalSize.isValid() && m_window)
        m_logicalSize = m_window->size();
    if (ensureChild()) {
        wl_display_flush(m_display);
        pollFrames();
    }
}

void WaylandVideoSurface::setGeometry(const QSize &logicalSize, qreal devicePixelRatio)
{
    if (logicalSize.isEmpty() || !std::isfinite(devicePixelRatio) || devicePixelRatio <= 0)
        return;
    m_logicalSize = logicalSize;
    kichi_embed_set_viewport(logicalSize.width(), logicalSize.height(), devicePixelRatio);
    if (m_viewport) {
        wp_viewport_set_destination(m_viewport, logicalSize.width(), logicalSize.height());
        if (m_child)
            wl_surface_commit(m_child);
        wl_display_flush(m_display);
    }
}

void WaylandVideoSurface::poll()
{
    dispatch();

    if (!m_active || !m_window || !m_window->isVisible())
        return;

    // 换屏不一定伴随 preferred_changed（有些合成器只在 surface 重新映射时更新），
    // 这里按 QWindow 当前 screen 兜底触发一次查询。
    QScreen *screen = m_window->screen();
    if (screen != m_screen) {
        m_screen = screen;
        m_preferredDirty = true;
    }

    if (!ensureChild())
        return;
    // 窗口还没 exposed 时先别清标记：第一次 poll 可能早于 Qt 提交首帧，
    // 这时查 preferred 会拿到默认 sRGB/80，等映射完成再查。
    if (m_preferredDirty && m_window->isExposed()) {
        m_preferredDirty = false;
        queryPlaybackPreferred();
    }
    pollFrames();
}

void WaylandVideoSurface::pollFrames()
{
    if (!m_child)
        return;
    if (!kichi_embed_frame_pending())
        return;
    KichiFrame *frame = kichi_embed_frame_take();
    if (!frame)
        return;
    commitFrame(frame);
}

bool WaylandVideoSurface::createDmabufBuffer(KichiFrame *frame, int width, int height,
                                             wl_buffer **out)
{
    const int planes = kichi_embed_frame_plane_count(frame);
    if (planes <= 0)
        return false;
    const uint32_t fourcc = kichi_embed_frame_fourcc(frame);
    const uint64_t modifier = kichi_embed_frame_modifier(frame);

    auto *params = zwp_linux_dmabuf_v1_create_params(m_dmabuf);
    if (!params)
        return false;
    wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(params), m_queue);

    for (int index = 0; index < planes; ++index) {
        const int fd = kichi_embed_frame_plane_fd(frame, index);
        const uint32_t offset = kichi_embed_frame_plane_offset(frame, index);
        const uint32_t stride = kichi_embed_frame_plane_stride(frame, index);
        if (fd < 0) {
            zwp_linux_buffer_params_v1_destroy(params);
            return false;
        }
        zwp_linux_buffer_params_v1_add(params, fd, uint32_t(index), offset, stride,
                                       uint32_t(modifier >> 32),
                                       uint32_t(modifier & 0xffffffffu));
    }

    *out = zwp_linux_buffer_params_v1_create_immed(params, width, height, fourcc, 0);
    zwp_linux_buffer_params_v1_destroy(params); // create_immed 不销毁 params
    return *out != nullptr;
}

bool WaylandVideoSurface::createShmBuffer(KichiFrame *frame, int width, int height,
                                          wl_buffer **out)
{
    const uint8_t *pixels = kichi_embed_frame_pixels(frame);
    if (!pixels)
        return false;

    const int stride = width * 4;
    const size_t size = size_t(stride) * size_t(height);
    const int fd = memfd_create("kichi-shm", MFD_CLOEXEC);
    if (fd < 0)
        return false;
    if (ftruncate(fd, off_t(size)) < 0) {
        close(fd);
        return false;
    }
    void *mapping = mmap(nullptr, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (mapping == MAP_FAILED) {
        close(fd);
        return false;
    }
    std::memcpy(mapping, pixels, size);
    munmap(mapping, size);

    auto *pool = wl_shm_create_pool(m_shm, fd, int(size));
    if (!pool) {
        close(fd);
        return false;
    }
    wl_display_flush(m_display);
    close(fd);

    *out = wl_shm_pool_create_buffer(pool, 0, width, height, stride, WL_SHM_FORMAT_XRGB8888);
    wl_shm_pool_destroy(pool);
    return *out != nullptr;
}

void WaylandVideoSurface::commitFrame(KichiFrame *frame)
{
    if (!m_child) {
        kichi_embed_frame_free(frame);
        return;
    }
    const int width = int(kichi_embed_frame_width(frame));
    const int height = int(kichi_embed_frame_height(frame));
    if (width <= 0 || height <= 0) {
        kichi_embed_frame_free(frame);
        return;
    }

    wl_buffer *buffer = nullptr;
    const bool shm = kichi_embed_frame_is_shm(frame);
    const bool ok = shm ? createShmBuffer(frame, width, height, &buffer)
                        : createDmabufBuffer(frame, width, height, &buffer);
    if (!ok || !buffer) {
        kichi_embed_frame_free(frame);
        setError(QStringLiteral("提交视频帧失败"));
        return;
    }

    m_frames[buffer] = frame;
    wl_buffer_add_listener(buffer, &bufferListener, this);

    // 给真实 child surface 请求 presentation feedback；token 由代理随帧传来。
    // 必须在 commit 前请求，事件才对应这次内容提交。
    const uint64_t presentationToken = kichi_embed_frame_presentation_token(frame);
    if (presentationToken != 0) {
        bool requested = false;
        if (m_presentation) {
            auto *feedback = wp_presentation_feedback(m_presentation, m_child);
            if (feedback) {
                wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(feedback), m_queue);
                wp_presentation_feedback_add_listener(feedback, &presentationFeedbackListener,
                                                      this);
                m_pendingFeedback[feedback] = presentationToken;
                requested = true;
            }
        }
        if (!requested) {
            // 没有协议 / 对象创建失败：明确 discarded，绝不让 mpv 等假事件。
            KichiPresentationEvent event{};
            event.token = presentationToken;
            event.presented = 0;
            pushPresentationEvent(event);
        }
    }

    wl_surface_attach(m_child, buffer, 0, 0);
    wl_surface_damage_buffer(m_child, 0, 0, width, height);
    wl_surface_commit(m_child);
    wl_display_flush(m_display);

    // 帧进合成器后就可以让 mpv 继续；真实 buffer 的释放由 release 事件回收。
    kichi_embed_frame_displayed(frame);
    if (!m_hasFrame) {
        m_hasFrame = true;
        notifyFrame();
    }
}

void WaylandVideoSurface::onBufferReleased(wl_buffer *buffer)
{
    auto it = m_frames.find(buffer);
    if (it != m_frames.end()) {
        if (it->second)
            kichi_embed_frame_free(it->second);
        m_frames.erase(it);
    }
    wl_buffer_destroy(buffer);
}

void WaylandVideoSurface::onPresentationPresented(struct wp_presentation_feedback *feedback,
                                                  uint32_t tvSecHi, uint32_t tvSecLo,
                                                  uint32_t tvNsec, uint32_t refresh,
                                                  uint32_t seqHi, uint32_t seqLo, uint32_t flags)
{
    auto it = m_pendingFeedback.find(feedback);
    if (it != m_pendingFeedback.end()) {
        KichiPresentationEvent event{};
        event.token = it->second;
        event.presented = 1;
        event.tv_sec_hi = tvSecHi;
        event.tv_sec_lo = tvSecLo;
        event.tv_nsec = tvNsec;
        event.refresh = refresh;
        event.seq_hi = seqHi;
        event.seq_lo = seqLo;
        event.flags = flags;
        pushPresentationEvent(event);
        m_pendingFeedback.erase(it);
    }
    wp_presentation_feedback_destroy(feedback);
}

void WaylandVideoSurface::onPresentationDiscarded(struct wp_presentation_feedback *feedback)
{
    auto it = m_pendingFeedback.find(feedback);
    if (it != m_pendingFeedback.end()) {
        KichiPresentationEvent event{};
        event.token = it->second;
        event.presented = 0;
        pushPresentationEvent(event);
        m_pendingFeedback.erase(it);
    }
    wp_presentation_feedback_destroy(feedback);
}

void WaylandVideoSurface::dispatch()
{
    if (!m_display || !m_queue)
        return;
    while (wl_display_dispatch_queue_pending(m_display, m_queue) > 0) {
    }
    wl_display_flush(m_display);
}

// ------------------------------------------------------------------- colors

void WaylandVideoSurface::applyColorDescription()
{
    if (!m_colorSurface || !m_colorManager)
        return;

    KichiColorTarget target{};
    bool valid = false;
    {
        std::lock_guard<std::mutex> lock(gTargetMutex);
        valid = gPreferredTargetValid;
        target = gPreferredTarget;
    }
    if (valid) {
        if (applyParametric(waylandPrimaries(target.primaries), waylandTransfer(target.transfer),
                            target.min_luminance_10000, target.max_luminance,
                            target.reference_luminance))
            return;
    }

    // 没有可用 target / 合成器不支持 parametric：回退到 child preferred。
    if (!m_childFeedback) {
        wp_color_management_surface_v1_unset_image_description(m_colorSurface);
        if (m_child) {
            wl_surface_commit(m_child);
            wl_display_flush(m_display);
        }
        return;
    }
    auto *description = wp_color_management_surface_feedback_v1_get_preferred(m_childFeedback);
    if (!description) {
        wp_color_management_surface_v1_unset_image_description(m_colorSurface);
        if (m_child) {
            wl_surface_commit(m_child);
            wl_display_flush(m_display);
        }
        return;
    }
    wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(description), m_queue);
    auto *state = new DescState{this};
    wp_image_description_v1_add_listener(description, &descListener, state);
}

bool WaylandVideoSurface::applyParametric(uint32_t primaries, uint32_t transfer,
                                          uint32_t minLuminance, uint32_t maxLuminance,
                                          uint32_t referenceLuminance)
{
    if (!m_colorSurface || !m_colorManager || !m_cmParametric || !m_cmSetPrimaries
        || !m_cmSetLuminances)
        return false;
    if (transfer == WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_ST2084_PQ && !m_cmHasPq)
        return false;
    if (transfer == WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_GAMMA22 && !m_cmHasGamma22)
        return false;
    if (transfer == WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_SRGB && !m_cmHasSrgb)
        return false;
    if (transfer == WP_COLOR_MANAGER_V1_TRANSFER_FUNCTION_HLG && !m_cmHasHlg)
        return false;

    auto *creator = wp_color_manager_v1_create_parametric_creator(m_colorManager);
    if (!creator)
        return false;
    wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(creator), m_queue);
    wp_image_description_creator_params_v1_set_primaries_named(creator, primaries);
    wp_image_description_creator_params_v1_set_tf_named(creator, transfer);
    wp_image_description_creator_params_v1_set_luminances(creator, minLuminance, maxLuminance,
                                                          referenceLuminance);
    auto *description = wp_image_description_creator_params_v1_create(creator);
    if (!description)
        return false;
    wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(description), m_queue);
    auto *state = new DescState{this};
    wp_image_description_v1_add_listener(description, &descListener, state);
    qInfo() << "[kichi] 内嵌 Wayland 色彩：preferred primaries=" << primaries
            << "transfer=" << transfer << "lum=" << minLuminance << "/" << maxLuminance
            << "/" << referenceLuminance;
    return true;
}

void WaylandVideoSurface::applyImageDescription(wp_image_description_v1 *description)
{
    if (!m_colorSurface) {
        wp_image_description_v1_destroy(description);
        return;
    }

    auto *info = wp_image_description_v1_get_information(description);
    if (info) {
        wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(info), m_queue);
        auto *state = new InfoState{this};
        wp_image_description_info_v1_add_listener(info, &infoListener, state);
    }

    wp_color_management_surface_v1_set_image_description(
        m_colorSurface, description, WP_COLOR_MANAGER_V1_RENDER_INTENT_PERCEPTUAL);
    wp_image_description_v1_destroy(description);

    // surface description 是双缓冲状态：commit 一次才生效，暂停 / 没有新帧时
    // 也能切换。
    if (m_child) {
        wl_surface_commit(m_child);
        wl_display_flush(m_display);
    }
}

bool WaylandVideoSurface::rememberPreferred(uint32_t primaries, uint32_t transfer,
                                            uint32_t minLuminance, uint32_t maxLuminance,
                                            uint32_t referenceLuminance)
{
    KichiColorTarget next{};
    next.primaries = mapPrimaries(primaries);
    next.transfer = mapTransfer(transfer);
    next.min_luminance_10000 = minLuminance;
    next.max_luminance = maxLuminance;
    next.reference_luminance = referenceLuminance;
    if (next.primaries == KICHI_PRIMARIES_UNKNOWN || next.transfer == KICHI_TRANSFER_UNKNOWN
        || next.max_luminance == 0)
        return false;

    {
        std::lock_guard<std::mutex> lock(gTargetMutex);
        if (gPreferredTargetValid && gPreferredTarget.primaries == next.primaries
            && gPreferredTarget.transfer == next.transfer
            && gPreferredTarget.min_luminance_10000 == next.min_luminance_10000
            && gPreferredTarget.max_luminance == next.max_luminance
            && gPreferredTarget.reference_luminance == next.reference_luminance)
            return false;
        gPreferredTarget = next;
        gPreferredTargetValid = true;
    }
    applyColorInfo(next);
    return true;
}

// C++ → Rust：只投递，不跨线程直接调 mpv。
void WaylandVideoSurface::publishTarget()
{
    KichiColorTarget target{};
    {
        std::lock_guard<std::mutex> lock(gTargetMutex);
        if (!gPreferredTargetValid)
            return;
        target = gPreferredTarget;
    }
    kichi_embed_set_color_target(&target);
}

void WaylandVideoSurface::applyColorInfo(const KichiColorTarget &target)
{
    QString primaries;
    switch (target.primaries) {
    case KICHI_PRIMARIES_BT2020:
        primaries = QStringLiteral("BT.2020");
        break;
    case KICHI_PRIMARIES_DISPLAY_P3:
        primaries = QStringLiteral("Display P3");
        break;
    case KICHI_PRIMARIES_BT709:
        primaries = QStringLiteral("BT.709");
        break;
    default:
        primaries = QStringLiteral("未知色域");
        break;
    }

    QString transfer;
    switch (target.transfer) {
    case KICHI_TRANSFER_PQ:
        transfer = QStringLiteral("PQ");
        break;
    case KICHI_TRANSFER_HLG:
        transfer = QStringLiteral("HLG");
        break;
    case KICHI_TRANSFER_SRGB:
        transfer = QStringLiteral("sRGB");
        break;
    case KICHI_TRANSFER_GAMMA22:
        transfer = QStringLiteral("gamma2.2");
        break;
    default:
        transfer = QStringLiteral("未知传输");
        break;
    }

    m_colorInfo = QStringLiteral("%1 / %2 / %3 nits")
                      .arg(primaries, transfer)
                      .arg(target.max_luminance);
}

// -------------------------------------------------------------------- misc

void WaylandVideoSurface::notifyFrame()
{
    if (m_onFrame)
        m_onFrame();
}

void WaylandVideoSurface::setError(const QString &error)
{
    if (m_error == error)
        return;
    m_error = error;
    qWarning().noquote() << "[kichi] 内嵌 Wayland 视频：" << error;
}
