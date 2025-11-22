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
#define PHY_ADDR    0x01
#define PHY_SMR     0x1F
static void PHY_ToggleLEDs();
static void PHY_EnableManualLEDMode();

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

static void PHY_EnableManualLEDMode()
{
	uint32_t smr;

	// Read SMR
	HAL_ETH_ReadPHYRegister(&heth, PHY_ADDR, PHY_SMR, &smr);

	// Enter manual mode
	smr &= ~(0b11 << 2);
	smr |=  (0b10 << 2);

	HAL_ETH_WritePHYRegister(&heth, PHY_ADDR, PHY_SMR, smr);
}

static void PHY_ToggleLEDs()
{
    uint32_t smr;

    // Read LED mode
    HAL_ETH_ReadPHYRegister(&heth, PHY_ADDR, PHY_SMR, &smr);

    uint32_t mode = (smr >> 2) & 0x3;
    if (mode == 0b11)
    {
        // Currently ON → force OFF
        smr &= ~(0b11 << 2);
        smr |=  (0b10 << 2);
    }
    else
    {
        // Currently OFF → force ON
        smr &= ~(0b11 << 2);
        smr |=  (0b11 << 2);
    }

    HAL_ETH_WritePHYRegister(&heth, PHY_ADDR, PHY_SMR, smr);
}
