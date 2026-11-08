#pragma once

// kichi 内嵌播放的 Rust C ABI（见 kichi/src/player/embed/ffi.rs）。
// KichiFrame 是不透明句柄：取走后必须用 kichi_embed_frame_free 归还，
// 期间可以用只读访问器查看格式。

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct KichiFrame KichiFrame;

// 合成器 preferred image description 的原始值（wp_image_description_info_v1）。
// 整数值与 kichi/src/player/embed/color.rs 的 Rust 枚举保持一致。
typedef struct KichiColorInfo {
    int32_t primaries;
    int32_t transfer;
    uint32_t min_luminance_10000;
    uint32_t max_luminance;
    uint32_t reference_luminance;
    // target_luminance 的 max；0 表示 compositor 没发。
    uint32_t target_max_luminance;
    uint32_t max_cll;
    uint32_t max_fall;
} KichiColorInfo;

// 归一化后的输出目标：mpv target-* 和视频 surface description 共用。
typedef struct KichiColorTarget {
    int32_t primaries;
    int32_t transfer;
    uint32_t min_luminance_10000;
    uint32_t max_luminance;
    uint32_t reference_luminance;
} KichiColorTarget;

// 把 preferred 原始描述换算成输出目标（mpv target 模式）。成功返回 true。
bool kichi_normalize_color_info(const KichiColorInfo *info, KichiColorTarget *out);

// C++ presenter 在真实 child surface 上收到的 wp_presentation feedback。
// token 由代理生成并随 KichiFrame 传来；presented=0 表示 discarded。
typedef struct KichiPresentationEvent {
    uint64_t token;
    uint32_t presented;
    uint32_t tv_sec_hi;
    uint32_t tv_sec_lo;
    uint32_t tv_nsec;
    uint32_t refresh;
    uint32_t seq_hi;
    uint32_t seq_lo;
    uint32_t flags;
} KichiPresentationEvent;

#define KICHI_PRIMARIES_UNKNOWN 0
#define KICHI_PRIMARIES_BT709 1
#define KICHI_PRIMARIES_BT2020 2
#define KICHI_PRIMARIES_DISPLAY_P3 3

#define KICHI_TRANSFER_UNKNOWN 0
#define KICHI_TRANSFER_GAMMA22 1
#define KICHI_TRANSFER_SRGB 2
#define KICHI_TRANSFER_PQ 3
#define KICHI_TRANSFER_HLG 4

// 合成器给这个 surface 的首选目标；启动时由 Wayland presenter 缓存。
bool kichi_wayland_preferred_target(KichiColorTarget *out);

// 播放窗口换屏 / 合成器 preferred 变化时，由 C++ presenter 投递新目标。
// Rust 只把它转给 mpv 事件线程，不在这里直接调 mpv。
void kichi_embed_set_color_target(const KichiColorTarget *target);

bool kichi_embed_frame_pending(void);
KichiFrame *kichi_embed_frame_take(void);
void kichi_embed_frame_free(KichiFrame *frame);
void kichi_embed_frame_displayed(KichiFrame *frame);

// 这一帧等待兑现的 presentation feedback token；0 = 没有。
uint64_t kichi_embed_frame_presentation_token(const KichiFrame *frame);

uint32_t kichi_embed_frame_width(const KichiFrame *frame);
uint32_t kichi_embed_frame_height(const KichiFrame *frame);
uint32_t kichi_embed_frame_fourcc(const KichiFrame *frame);
uint64_t kichi_embed_frame_modifier(const KichiFrame *frame);
bool kichi_embed_frame_is_shm(const KichiFrame *frame);
const uint8_t *kichi_embed_frame_pixels(const KichiFrame *frame);

int kichi_embed_frame_plane_count(const KichiFrame *frame);
int kichi_embed_frame_plane_fd(const KichiFrame *frame, int index);
uint32_t kichi_embed_frame_plane_offset(const KichiFrame *frame, int index);
uint32_t kichi_embed_frame_plane_stride(const KichiFrame *frame, int index);

void kichi_embed_set_viewport(int width, int height, double scale);

// 把帧按比例放进 item，out = { x, y, width, height }；尺寸无效时返回 false。
bool kichi_embed_fit_rect(double item_width, double item_height, double frame_width,
                          double frame_height, double out[4]);

#ifdef __cplusplus
}
#endif
