"""Diffie-Hellman test material. Published constants, nothing secret.

``SAFE_PRIME`` is RFC 3526's 2048-bit MODP group (group 14) - a published safe
prime that this project did not choose or generate, which is what makes it useful
here: the checks in §1.2 have to accept a real one, and a prime we produced
ourselves would only prove our generator agrees with our checker.

It satisfies every §1.2 condition: 2048 bits, ``2^2047 < p < 2^2048``, ``p`` and
``(p-1)/2`` both prime, and ``p mod 8 == 7`` so ``g = 2`` is a quadratic residue.
"""

SAFE_PRIME = int(
    "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74"
    "020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F1437"
    "4FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED"
    "EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF05"
    "98DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB"
    "9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3B"
    "E39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF695581718"
    "3995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF",
    16,
)

# 2048 bits and in range, but divisible by 3 - so it fails the primality test
# immediately rather than after fifteen Miller-Rabin rounds.
COMPOSITE_2048 = 2**2047 + 1
