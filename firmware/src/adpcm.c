/* SPDX-License-Identifier: MIT */
/* IMA ADPCM encoder. Must match core/src/adpcm.rs, which core/tests/adpcm_c.rs checks. */

#include "adpcm.h"

static const int8_t index_table[16] = {-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8};

static const int16_t step_table[89] = {
	7,     8,     9,     10,    11,    12,    13,    14,    16,    17,    19,    21,    23,
	25,    28,    31,    34,    37,    41,    45,    50,    55,    60,    66,    73,    80,
	88,    97,    107,   118,   130,   143,   157,   173,   190,   209,   230,   253,   279,
	307,   337,   371,   408,   449,   494,   544,   598,   658,   724,   796,   876,   963,
	1060,  1166,  1282,  1411,  1552,  1707,  1878,  2066,  2272,  2499,  2749,  3024,  3327,
	3660,  4026,  4428,  4871,  5358,  5894,  6484,  7132,  7845,  8630,  9493,  10442, 11487,
	12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
};

size_t stv_adpcm_encode(const int16_t *pcm, size_t count, uint8_t *index, uint8_t *out)
{
	int32_t predictor = count > 0 ? pcm[0] : 0;
	int32_t idx = *index;
	size_t len = STV_ADPCM_STATE_LEN;

	out[0] = (uint8_t)(predictor & 0xFF);
	out[1] = (uint8_t)((predictor >> 8) & 0xFF);
	out[2] = *index;

	for (size_t i = 0; i < count; i++) {
		int32_t step = step_table[idx];
		int32_t delta = pcm[i] - predictor;
		int32_t diff = step >> 3;
		uint8_t nibble = 0;

		if (delta < 0) {
			nibble = 8;
			delta = -delta;
		}
		if (delta >= step) {
			nibble |= 4;
			delta -= step;
			diff += step;
		}
		if (delta >= (step >> 1)) {
			nibble |= 2;
			delta -= step >> 1;
			diff += step >> 1;
		}
		if (delta >= (step >> 2)) {
			nibble |= 1;
			diff += step >> 2;
		}

		predictor += (nibble & 8) ? -diff : diff;
		if (predictor > 32767) {
			predictor = 32767;
		} else if (predictor < -32768) {
			predictor = -32768;
		}
		idx += index_table[nibble];
		if (idx < 0) {
			idx = 0;
		} else if (idx > 88) {
			idx = 88;
		}

		if ((i & 1) == 0) {
			out[len++] = nibble;
		} else {
			out[len - 1] |= (uint8_t)(nibble << 4);
		}
	}
	*index = (uint8_t)idx;
	return len;
}
