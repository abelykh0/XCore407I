#ifndef _DEMO_COLORS_H_
#define _DEMO_COLORS_H_

#include <stdint.h>

#define CONVERT_6BIT_TO_DDD(v) ( \
    (uint16_t)((((v) & 0x03) * 100) + \
               ((((v) >> 2) & 0x03) * 10) + \
               (((v) >> 4) & 0x03)) \
)

#ifdef __cplusplus
extern "C" {
#endif

void init_demo_colors();
int32_t loop_demo_colors();

#ifdef __cplusplus
}
#endif

#endif
