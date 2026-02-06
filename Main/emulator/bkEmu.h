#ifndef __BKEMU_H
#define __BKEMU_H

#include <stdint.h>
#include "defines.h"
#include "bkScreen.h"

#define RAM_AVAILABLE 0x4000
extern uint8_t RamBuffer[RAM_AVAILABLE];
extern bk::BkScreen* _bkScreen;
extern pdp_regs pdp;

void bk_setup(bk::BkScreen* bkScreen);
int32_t bk_loop();
void bk_reset();

#endif
