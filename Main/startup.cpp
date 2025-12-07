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
static void USB_DEVICE_Init();
extern USBD_HandleTypeDef hUsbDeviceHS;
uint8_t cdc_ep[3] = { CDC_IN_EP, CDC_OUT_EP, CDC_CMD_EP };
uint8_t video_ep[1] = { UVC_IN_EP };
#define CDC_CLASS_ID 1

// Using Ethernet PHY to toggle LEDs
#define PHY_ADDR  0x01
#define PHY_LEDCR 0x18
static void PHY_EnableManualLEDMode();
static void PHY_ToggleLEDs();

// demo
Display::Screen Screen;
static uint8_t GetUsbBuffer(char *buffer, uint8_t maxLength);
static uint8_t utf8_to_cp866(const uint8_t* utf8);

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
	//Clear(0b00101010);
}

extern "C" void loop()
{
	//uint8_t test_msg[] = "STM32 CDC Test\n";
	//CDC_Transmit_HS(test_msg, sizeof(test_msg) - 1);

	//loop_demo_colors();
	//PHY_ToggleLEDs();
	//HAL_Delay(1000);

	char buffer[32];
    uint8_t len = GetUsbBuffer(buffer, 32);
	//uint8_t len = 0;
    if (len == 0)
    {
    	return;
    }

    if (len == 1)
	{
    	Screen.PrintCharacter(buffer[0], 0x3F10);
    	return;
	}

    if (len == 2)
    {
    	uint8_t cyrillicChar = utf8_to_cp866((const uint8_t*)buffer);
    	if (cyrillicChar != 0)
    	{
        	Screen.PrintCharacter(cyrillicChar, 0x3F10);
    	}

    	return;
    }

	if (len == 3 && buffer[0] == '\e' && buffer[1] == '[')
	{
		switch (buffer[2])
		{
		case 'D':
			if (Screen.cursor_x > 0)
			{
				Screen.SetCursorPosition(Screen.cursor_x - 1, Screen.cursor_y);
			}
			break;
		case 'C':
			if (Screen.cursor_x < TEXT_COLUMNS - 1)
			{
				Screen.SetCursorPosition(Screen.cursor_x + 1, Screen.cursor_y);
			}
			break;
		case 'A':
			if (Screen.cursor_y > 0)
			{
				Screen.SetCursorPosition(Screen.cursor_x, Screen.cursor_y - 1);
			}
			break;
		case 'B':
			if (Screen.cursor_y < TEXT_ROWS - 1)
			{
				Screen.SetCursorPosition(Screen.cursor_x, Screen.cursor_y + 1);
			}
			break;
		}
	}
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

static uint8_t GetUsbBuffer(char *buffer, uint8_t maxLength)
{
	USBD_CDC_HandleTypeDef *hcdc = (USBD_CDC_HandleTypeDef*)hUsbDeviceHS.pClassDataCmsit[CDC_CLASS_ID];
	if (hcdc == NULL)
	{
		return 0;
	}

    uint32_t len = hcdc->RxLength;
    if (len > 0)
    {
        if (len > maxLength)
        {
            len = maxLength;
        }

        memcpy(buffer, hcdc->RxBuffer, len);
        hcdc->RxLength = 0;
    }

    return (uint8_t)len;
}

static uint8_t utf8_to_cp866(const uint8_t* utf8)
{
    uint8_t hi = utf8[0];
    uint8_t lo = utf8[1];

    // Uppercase А–Я: D0 90 .. D0 AF -> E0 .. EF
    if (hi == 0xD0 && lo >= 0x90 && lo <= 0xAF)
    {
        return 0x80 + (lo - 0x90);
    }

    // Lowercase а–п: D0 B0 .. D0 BF -> 80 .. 8F
    if (hi == 0xD0 && lo >= 0xB0 && lo <= 0xBF)
    {
        return 0xA0 + (lo - 0xB0);
    }

    // Lowercase р–я: D1 80 .. D1 8F -> 90 .. 9F
    if (hi == 0xD1 && lo >= 0x80 && lo <= 0x8F)
    {
        return 0xE0 + (lo - 0x80);
    }

    return 0; // not Cyrillic
}
