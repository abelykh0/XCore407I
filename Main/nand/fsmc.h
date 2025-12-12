#ifndef __FSMC_H
#define __FSMC_H

#include "stm32f4xx_hal_def.h"
#include "stm32f4xx_ll_fsmc.h"
#include "stm32f4xx_hal_nand.h"

#ifdef __cplusplus
 extern "C" {
#endif

extern NAND_HandleTypeDef hnand1;

void MX_FSMC_Init(void);
void HAL_NAND_MspInit(NAND_HandleTypeDef* hnand);
void HAL_NAND_MspDeInit(NAND_HandleTypeDef* hnand);

#ifdef __cplusplus
}
#endif
#endif /*__FSMC_H */
