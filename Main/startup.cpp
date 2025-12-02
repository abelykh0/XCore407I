#include "startup.h"
#include "stm32f4xx_hal.h"
#include "eth.h"
#include "stm32f4xx_hal_eth.h"

#include "usb_device.h"
#include "usbd_desc.h"
#include "usbd_cdc_if.h"
#include "usbd_video_if.h"

#include "screen/screen.h"
#include "demo_colors/demo_colors.h"

// USB HS Device
extern USBD_ClassTypeDef USBD_CMPSIT;
static void USB_DEVICE_Init();
extern USBD_HandleTypeDef hUsbDeviceHS;
extern PCD_HandleTypeDef hpcd_USB_OTG_HS;
uint8_t cdc_ep[3] = { CDC_IN_EP, CDC_OUT_EP, CDC_CMD_EP };
uint8_t video_ep[1] = { UVC_IN_EP };

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

	init_demo_colors();
}

extern "C" void loop()
{
	loop_demo_colors();

	PHY_ToggleLEDs();
	//uint8_t test_msg[] = "STM32 CDC Test\n";
	//CDC_Transmit_HS(test_msg, sizeof(test_msg) - 1);

	HAL_Delay(1000);
}

static void USB_DEVICE_Init()
{
	//MX_USB_DEVICE_Init();

	if (USBD_Init(&hUsbDeviceHS, &HS_Desc, DEVICE_HS) != USBD_OK)
	{
		Error_Handler();
	}

	// UVC
	if (USBD_VIDEO_RegisterInterface(&hUsbDeviceHS, &USBD_VIDEO_fops_HS) != USBD_OK)
	{
		Error_Handler();
	}
	if (USBD_RegisterClassComposite(&hUsbDeviceHS, &USBD_VIDEO, CLASS_TYPE_VIDEO, video_ep) != USBD_OK)
	{
		Error_Handler();
	}

	// CDC
	if (USBD_CDC_RegisterInterface(&hUsbDeviceHS, &USBD_Interface_fops_HS) != USBD_OK)
	{
		Error_Handler();
	}
	if (USBD_RegisterClassComposite(&hUsbDeviceHS, &USBD_CDC, CLASS_TYPE_CDC, cdc_ep) != USBD_OK)
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
