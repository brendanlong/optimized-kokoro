// Oracle driver: reads records "voice\0text\0" from stdin, writes the clauses
// espeak_TextToPhonemes returns (phonemizer's mode: IPA, tie U+0361), each
// followed by '\n', then '\0' per record.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <locale.h>
#include <espeak-ng/speak_lib.h>
#include <espeak-ng/espeak_ng.h>
#include "translate.h"
#include "phoneme.h"
#include "synthesize.h"
#if __has_include(<valgrind/valgrind.h>)
#include <valgrind/valgrind.h>
#else
#define VALGRIND_COUNT_ERRORS 0
#endif

static void dump_phoneme_list(void) {
	for (int ix = 0; ix < n_phoneme_list; ix++) {
		PHONEME_LIST *p = &phoneme_list[ix];
		unsigned int m = p->ph ? p->ph->mnemonic : 0;
		fprintf(stderr, "  [%d] code=%d ph=%.4s phcode=%d type=%d stress=%d wstress=%d flags=%x src=%d new=%d tone=%d\n",
			ix, p->ph ? p->ph->code : -1, (char *)&m, p->phcode, p->type, p->stresslevel, p->wordstress,
			p->synthflags, p->sourceix, p->newword, p->tone_ph);
	}
}

static char *read_field(void) {
	size_t cap = 256, n = 0;
	char *buf = malloc(cap);
	int c;
	while ((c = getchar()) != EOF && c != 0) {
		if (n + 1 >= cap) buf = realloc(buf, cap *= 2);
		buf[n++] = (char)c;
	}
	if (c == EOF && n == 0) { free(buf); return NULL; }
	buf[n] = 0;
	return buf;
}

int main(int argc, char **argv) {
	setlocale(LC_CTYPE, "");
	const char *data = argc > 1 ? argv[1] : NULL;
	if (espeak_Initialize(AUDIO_OUTPUT_SYNCHRONOUS, 0, data, 0) <= 0) {
		fprintf(stderr, "init failed\n");
		return 1;
	}
	char current[256] = "";
	int mode = 0x02 | 0x01 << 7 | 0x361 << 8;
	if (getenv("ESPEAK_TRACE"))
		espeak_SetPhonemeTrace(espeakPHONEMES_TRACE, stderr);
	for (int record = 0;; record++) {
		// Marks where each record's sanitizer reports start (see oracle.py --ub).
		if (getenv("TTP_MARK_RECORDS"))
			fprintf(stderr, "\n@@ttp-record %d\n", record);
		char *voice = read_field();
		if (!voice) break;
		char *text = read_field();
		if (!text) text = strdup("");
		if (strcmp(voice, current) != 0) {
			if (espeak_SetVoiceByName(voice) != 0) fprintf(stderr, "voice %s failed\n", voice);
			snprintf(current, sizeof current, "%s", voice);
		}
		const void *p = text;
		while (p != NULL) {
			const char *ph = espeak_TextToPhonemes(&p, espeakCHARS_UTF8, mode);
			if (getenv("ESPEAK_DUMP"))
				dump_phoneme_list();
			fputs(ph ? ph : "", stdout);
			putchar('\n');
		}
		putchar(0);
		fflush(stdout);
		// memcheck reports each distinct error once, but counts every occurrence.
		if (getenv("TTP_MARK_RECORDS"))
			fprintf(stderr, "\n@@ttp-errors %d\n", (int)VALGRIND_COUNT_ERRORS);
		free(voice);
		free(text);
	}
	return 0;
}
