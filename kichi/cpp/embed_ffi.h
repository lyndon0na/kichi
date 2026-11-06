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
