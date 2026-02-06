#ifndef __STARTUP_H__
#define __STARTUP_H__

#ifdef __cplusplus
extern "C" {
#endif

#define FMC_Bank2_3
#include "main.h"

void initialize();
void setup();
void loop();
void TimerCallback();

#ifdef __cplusplus
}
#endif

#endif /* __STARTUP_H__ */
