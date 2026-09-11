"""Offline tests for Polymarket US order-book normalization."""

import unittest

from main import normalize_market_book


class MarketBookNormalizationTests(unittest.TestCase):
    def test_normalizes_and_sorts_native_long_book(self) -> None:
        book = normalize_market_book(
            {
                "marketData": {
                    "marketSlug": "nba-example",
                    "state": "open",
                    "transactTime": "2026-09-11T19:37:00Z",
                    "bids": [
                        {"px": {"value": "0.40", "currency": "USD"}, "qty": "2"},
                        {"px": {"value": "0.45", "currency": "USD"}, "qty": "1"},
                    ],
                    "offers": [
                        {"px": {"value": "0.55", "currency": "USD"}, "qty": "1"},
                        {"px": {"value": "0.50", "currency": "USD"}, "qty": "2"},
                    ],
                }
            },
            "nba-example",
        )

        self.assertTrue(book.success)
        self.assertEqual([level.price for level in book.bids], [0.45, 0.40])
        self.assertEqual([level.price for level in book.offers], [0.50, 0.55])

    def test_rejects_mismatched_market_slug(self) -> None:
        with self.assertRaisesRegex(ValueError, "did not match"):
            normalize_market_book(
                {
                    "marketData": {
                        "marketSlug": "wrong-market",
                        "state": "open",
                        "bids": [],
                        "offers": [],
                    }
                },
                "expected-market",
            )


if __name__ == "__main__":
    unittest.main()
