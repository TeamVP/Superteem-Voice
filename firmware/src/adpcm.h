#ifndef STV_ADPCM_H
#define STV_ADPCM_H

#include <stddef.h>
#include <stdint.h>

#define STV_ADPCM_STATE_LEN 3

/* Encodes one frame to protocol/PROTOCOL-v0.md layout and returns the payload length.
 * `index` keeps the step size adapted between frames. `out` needs 3 + (count + 1) / 2 bytes.
 */
size_t stv_adpcm_encode(const int16_t *pcm, size_t count, uint8_t *index, uint8_t *out);

#endif
