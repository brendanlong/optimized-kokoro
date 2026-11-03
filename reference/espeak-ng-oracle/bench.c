// Times espeak_TextToPhonemes over records "voice\0text\0" (one voice per run): bench DATA REPEATS < records
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <espeak-ng/speak_lib.h>

int main(int argc, char **argv) {
	if (espeak_Initialize(AUDIO_OUTPUT_SYNCHRONOUS, 0, argv[1], 0) <= 0) return 1;
	int repeats = argc > 2 ? atoi(argv[2]) : 3;
	size_t cap = 1 << 20, n = 0;
	char *buf = malloc(cap);
	int c;
	while ((c = getchar()) != EOF) {
		if (n + 1 >= cap) buf = realloc(buf, cap *= 2);
		buf[n++] = (char)c;
	}
	size_t ntexts = 0;
	char **texts = malloc(sizeof(char *) * (n / 2 + 1));
	char *voice = NULL;
	for (size_t i = 0; i < n;) {
		char *v = &buf[i];
		i += strlen(v) + 1;
		if (i >= n) break;
		char *t = &buf[i];
		i += strlen(t) + 1;
		voice = v;
		texts[ntexts++] = t;
	}
	if (!voice || espeak_SetVoiceByName(voice) != 0) return 1;
	int mode = 0x02 | 0x01 << 7 | 0x361 << 8;
	size_t sink = 0;
	struct timespec a, b;
	double best = 1e30;
	for (int r = 0; r < repeats; r++) {
		clock_gettime(CLOCK_MONOTONIC, &a);
		for (size_t k = 0; k < ntexts; k++) {
			const void *p = texts[k];
			while (p != NULL) {
				const char *ph = espeak_TextToPhonemes(&p, espeakCHARS_UTF8, mode);
				sink += ph ? strlen(ph) : 0;
			}
		}
		clock_gettime(CLOCK_MONOTONIC, &b);
		double s = (b.tv_sec - a.tv_sec) + (b.tv_nsec - a.tv_nsec) / 1e9;
		if (s < best) best = s;
	}
	printf("%s %zu calls %.3f s %.2f us/call (sink %zu)\n", voice, ntexts, best, best * 1e6 / ntexts, sink);
	return 0;
}
