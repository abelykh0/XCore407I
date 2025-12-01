#include <usbd_video_if.h>

static int8_t VIDEO_Itf_Init(void);
static int8_t VIDEO_Itf_DeInit(void);
static int8_t VIDEO_Itf_Control(uint8_t cmd, uint8_t *pbuf, uint16_t length);
static int8_t VIDEO_Itf_Data(uint8_t **pbuf, uint16_t *psize, uint16_t *pcktidx);

USBD_VIDEO_ItfTypeDef USBD_VIDEO_fops_HS =
{
  VIDEO_Itf_Init,
  VIDEO_Itf_DeInit,
  VIDEO_Itf_Control,
  VIDEO_Itf_Data,
};

/* Private functions ---------------------------------------------------------*/
/**
  * @brief  Initializes the VIDEO media low layer over USB FS IP
  * @param  VIDEOFreq: VIDEO frequency used to play the VIDEO stream.
  * @param  Volume: Initial volume level (from 0 (Mute) to 100 (Max))
  * @param  options: Reserved for future use
  * @retval USBD_OK if all operations are OK else USBD_FAIL
  */
static int8_t VIDEO_Itf_Init(void)
{
  /*
     Add your initialization code here
  */

  return USBD_OK;
}

/**
  * @brief  TEMPLATE_DeInit
  *         DeInitializes the UVC media low layer
  * @param  None
  * @retval Result of the operation: USBD_OK if all operations are OK else USBD_FAIL
  */
static int8_t VIDEO_Itf_DeInit(void)
{
  /*
     Add your deinitialization code here
  */
  return USBD_OK;
}


/**
  * @brief  TEMPLATE_Control
  *         Manage the UVC class requests
  * @param  Cmd: Command code
  * @param  Buf: Buffer containing command data (request parameters)
  * @param  Len: Number of data to be sent (in bytes)
  * @retval Result of the operation: USBD_OK if all operations are OK else USBD_FAIL
  */
static int8_t VIDEO_Itf_Control(uint8_t cmd, uint8_t *pbuf, uint16_t length)
{
  UNUSED(cmd);
  UNUSED(pbuf);
  UNUSED(length);

  return USBD_OK;
}

/**
  * @brief  TEMPLATE_Data
  *         Manage the UVC data packets
  * @param  pbuf: pointer to the buffer data to be filled
  * @param  psize: pointer tot he current packet size to be filled
  * @param  pcktidx: pointer to the current packet index in the current image
  * @retval Result of the operation: USBD_OK if all operations are OK else USBD_FAIL
  */
static int8_t VIDEO_Itf_Data(uint8_t **pbuf, uint16_t *psize, uint16_t *pcktidx)
{
  return USBD_OK;
}
