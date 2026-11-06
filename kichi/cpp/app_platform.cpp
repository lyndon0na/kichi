#include "app_platform.h"

#include <QtGui/QGuiApplication>

extern "C" void kichi_set_quit_on_last_window_closed(bool enabled)
{
    QGuiApplication::setQuitOnLastWindowClosed(enabled);
}
