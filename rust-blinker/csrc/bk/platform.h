#ifndef BK_PLATFORM_H
#define BK_PLATFORM_H

#include <stdint.h>
#include "defines.h"

#define BK_RAM_SIZE       0x4000u
#define BK_MONITOR_BASE   0x8000u
#define BK_MONITOR_SIZE   0x2000u
#define BK_BASIC_BASE     0xA000u
#define BK_BASIC_SIZE     0x5F80u

extern uint8_t RamBuffer[BK_RAM_SIZE];
extern pdp_regs pdp;
extern flag_t bkmodel;
extern flag_t io_stop_happened;
extern unsigned short last_branch;

extern uint8_t port0177660;
extern uint8_t port0177662;
extern uint16_t port0177716;
extern uint16_t port0177664;

void bk_platform_init(const uint8_t *monitor_rom,
                      const uint8_t *basic_rom,
                      uint32_t basic_size_bytes);
void bk_reset(void);
int32_t bk_loop(void);

#endif
