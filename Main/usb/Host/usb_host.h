#ifndef __USB_HOST__H__
#define __USB_HOST__H__

#ifdef __cplusplus
 extern "C" {
#endif

/* Includes ------------------------------------------------------------------*/
#include "stm32f4xx.h"
#include "stm32f4xx_hal.h"
#include "usbh_core.h"
#include "usbh_hid.h"

extern USBH_HandleTypeDef hUsbHostFS;

/** Status of the application. */
typedef enum {
  APPLICATION_IDLE = 0,
  APPLICATION_START,
  APPLICATION_READY,
  APPLICATION_DISCONNECT
} ApplicationTypeDef;

/** @brief USB Host initialization function. */
void MX_USB_HOST_Init(void);

void MX_USB_HOST_Process(void);

#ifdef __cplusplus
}
#endif

#endif /* __USB_HOST__H__ */

