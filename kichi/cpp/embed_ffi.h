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

// Wayland presenter 查询到的合成器首选色彩目标。整数值与
// kichi/src/player/embed/color.rs 的 Rust 枚举保持一致。
typedef struct KichiColorTarget {
    int32_t primaries;
    int32_t transfer;
    uint32_t min_luminance_10000;
    uint32_t max_luminance;
    uint32_t reference_luminance;
} KichiColorTarget;

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

bool kichi_embed_frame_pending(void);
KichiFrame *kichi_embed_frame_take(void);
void kichi_embed_frame_free(KichiFrame *frame);
void kichi_embed_frame_displayed(KichiFrame *frame);

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
