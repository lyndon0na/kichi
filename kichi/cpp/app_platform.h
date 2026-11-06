#pragma once

// 播放窗口是独立顶层窗口：主窗口缩到托盘后关掉它，不应该结束整个进程。
extern "C" void kichi_set_quit_on_last_window_closed(bool enabled);
