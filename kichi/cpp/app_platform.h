#pragma once

// 播放窗口是独立顶层窗口：主窗口缩到托盘后关掉它，不应该结束整个进程。
extern "C" void kichi_set_quit_on_last_window_closed(bool enabled);

// 应用系统外观：scheme 为 -1 表示没有明确偏好（沿用当前调色板），
// 0 = 浅色，1 = 深色；hasAccent 为 1 时把 accent 写进高亮 / 链接 / Accent。
// 只应在 GUI 线程调用。
extern "C" void kichi_apply_appearance(int scheme, int hasAccent,
                                       double accentRed, double accentGreen,
                                       double accentBlue);
