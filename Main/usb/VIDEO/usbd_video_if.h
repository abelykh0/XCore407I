/**
  ******************************************************************************
  * @file    usbd_video_if_template.h
  * @author  MCD Application Team
  * @brief   Template Header file for the video Interface application layer functions
  ******************************************************************************
  * @attention
  *
  * Copyright (c) 2020 STMicroelectronics.
  * All rights reserved.
  *
  * This software is licensed under terms that can be found in the LICENSE file
  * in the root directory of this software component.
  * If no LICENSE file comes with this software, it is provided AS-IS.
  *
  ******************************************************************************
  */

/* Define to prevent recursive inclusion -------------------------------------*/
#ifndef __USBD_VIDEO_IF_H__
#define __USBD_VIDEO_IF_H__

#ifdef __cplusplus
extern "C" {
#endif

/* Includes ------------------------------------------------------------------*/
#include "usbd_video.h"

/** VIDEO_IF Interface callback. */
extern USBD_VIDEO_ItfTypeDef USBD_VIDEO_fops_FS;
/**
  * @brief  Manages the DMA full transfer complete event.
  * @retval None
  */
void TransferComplete_CallBack_FS(void);

/**
  * @brief  Manages the DMA half transfer complete event.
  * @retval None
  */
void HalfTransfer_CallBack_FS(void);

/* Time laps between video frames in ms.
   Please adjust this value depending on required speed.
   Please note that this define uses the system HAL_Delay() which uses the systick.
   In case of changes on HAL_Delay, please ensure the values in ms correspond. */
#ifdef USE_USB_HS
#define USBD_VIDEO_IMAGE_LAPS                     160U
#else
#define USBD_VIDEO_IMAGE_LAPS                     80U
#endif /* USE_USB_HS */

/* USER CODE BEGIN EXPORTED_FUNCTIONS */

/* USER CODE END EXPORTED_FUNCTIONS */

/**
  * @}
  */

/**
  * @}
  */

/**
  * @}
  */

#ifdef __cplusplus
}
#endif

#endif /* USBD_VIDEO_IF_H_ */

