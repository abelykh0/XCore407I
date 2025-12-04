#ifndef COPY_WORDS_H
#define COPY_WORDS_H

/*
 * Moves some number of aligned words using the fastest method I could think up.
 */
void copy_words(const uint32_t* source,
		uint32_t* dest,
		uint32_t count);

#endif  // COPY_WORDS_H
