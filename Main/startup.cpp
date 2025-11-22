#include "stm32f4xx_hal.h"
#include "usbd_core.h"
#include "usbd_desc.h"
#include "usbd_video_if.h"

#include "startup.h"
#include "screen/screen.h"

extern USBD_HandleTypeDef hUsbDeviceHS;
extern PCD_HandleTypeDef hpcd_USB_OTG_HS;

static void USB_DEVICE_Init();

extern "C" void initialize()
{
}

extern "C" void setup()
{
	USB_DEVICE_Init();
}

extern "C" void loop()
{
}

void USB_DEVICE_Init()
{
	  if (USBD_Init(&hUsbDeviceHS, &HS_Desc, DEVICE_HS) != USBD_OK)
	  {
	    Error_Handler();
	  }

	  // Defaults are 512, 128, 372 (1,012 total)
	  HAL_PCDEx_SetRxFiFo(&hpcd_USB_OTG_HS,   128);
	  HAL_PCDEx_SetTxFiFo(&hpcd_USB_OTG_HS, 0, 96);
	  HAL_PCDEx_SetTxFiFo(&hpcd_USB_OTG_HS, 1, 788);

	  if (USBD_RegisterClass(&hUsbDeviceHS, &USBD_VIDEO) != USBD_OK)
	  {
	    Error_Handler();
	  }
	  if (USBD_VIDEO_RegisterInterface(&hUsbDeviceHS, &USBD_VIDEO_fops_FS) != USBD_OK)
	  {
	    Error_Handler();
	  }
	  if (USBD_Start(&hUsbDeviceHS) != USBD_OK)
	  {
	    Error_Handler();
	  }
}
