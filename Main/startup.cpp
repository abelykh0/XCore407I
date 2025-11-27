#include "stm32f4xx_hal.h"
#include "eth.h"
#include "stm32f4xx_hal_eth.h"
#include "usbd_core.h"
#include "usbd_desc.h"
#include "usbd_video_if.h"

#include "startup.h"
#include "screen/screen.h"

// USB HS Device
static void USB_DEVICE_Init();
extern USBD_HandleTypeDef hUsbDeviceHS;
extern PCD_HandleTypeDef hpcd_USB_OTG_HS;

// Using Ethernet PHY to toggle LEDs
#define PHY_ADDR  0x01
#define PHY_LEDCR 0x18
static void PHY_EnableManualLEDMode();
static void PHY_ToggleLEDs();

extern "C" void initialize()
{
}

extern "C" void setup()
{
	USB_DEVICE_Init();

	// Using Ethernet PHY to toggle LEDs
	MX_ETH_Init();
	PHY_EnableManualLEDMode();
}

extern "C" void loop()
{
	PHY_ToggleLEDs();
	HAL_Delay(500);
}

static void USB_DEVICE_Init()
{
	  if (USBD_Init(&hUsbDeviceHS, &HS_Desc, DEVICE_HS) != USBD_OK)
	  {
	    Error_Handler();
	  }

	  // Defaults are 512, 128, 372 (1,012 total)
	  HAL_PCDEx_SetRxFiFo(&hpcd_USB_OTG_HS,    128);
	  HAL_PCDEx_SetTxFiFo(&hpcd_USB_OTG_HS, 0,  96);
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

static void PHY_EnableManualLEDMode()
{
	// Enter manual mode
	uint32_t phycr = 0x38;
	HAL_ETH_WritePHYRegister(&heth, PHY_ADDR, PHY_LEDCR, phycr);
}

static void PHY_ToggleLEDs()
{
	static bool isOn = false;
    uint32_t phycr;

    // Read current PHY control register
    HAL_ETH_ReadPHYRegister(&heth, PHY_ADDR, PHY_LEDCR, &phycr);

    if (isOn)
    {
    	phycr &= ~0x03;
    }
    else
    {
    	phycr |= 0x03;
    }

    isOn = !isOn;

    // Write back the updated value
    HAL_ETH_WritePHYRegister(&heth, PHY_ADDR, PHY_LEDCR, phycr);
}
