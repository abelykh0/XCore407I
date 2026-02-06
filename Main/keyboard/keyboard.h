#ifndef _KEYBOARD_H
#define _KEYBOARD_H

#include "stdint.h"
#include "usb_host.h"

extern HID_KEYBD_Info_TypeDef lastInfo;

uint8_t GetScanCode(bool wait);

uint8_t GetAsciiCode();

#endif
