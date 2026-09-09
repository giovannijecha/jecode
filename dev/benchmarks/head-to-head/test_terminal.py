"""Parser correctness is independent of provider timing and PTY availability."""

import codecs
import unittest
from terminal import Terminal, MAX_PENDING


def screen():
    value = Terminal.__new__(Terminal)
    value.rows, value.cols = 40, 140
    value.lines = [[' '] * value.cols for _ in range(value.rows)]
    value.row = value.col = 0
    value.pending = ''
    value.replies = []
    value.send = value.replies.append
    return value


def state(value):
    return value.text(), value.row, value.col, value.pending, value.replies


class ParserTests(unittest.TestCase):
    def test_ascii_and_cursor_commands_have_expected_cells(self):
        value = screen()
        value.feed('first\x1b[2;3Hsecond\x1b[1;2HZ\x1b[6n')
        self.assertEqual(''.join(value.lines[0][:5]), 'fZrst')
        self.assertEqual(''.join(value.lines[1][2:8]), 'second')
        self.assertEqual(value.replies, ['\x1b[1;3R'])
        value.feed('\x1b[2J\x1b[Hnew\x1b[K')
        self.assertEqual(value.text().strip(), 'new')

    def test_every_escape_split_preserves_the_same_screen(self):
        for payload in ('one\x1b[2;3Htwo\x1b[31mred\x1b[0m',
                        'one\x1b]0;title\x07two', 'one\x1b]0;title\x1b\\two',
                        'a\rbc\nD\tE\x1b[6n\x1b[c\x1b[?u'):
            expected = screen()
            expected.feed(payload)
            for split in range(len(payload) + 1):
                with self.subTest(payload=repr(payload), split=split):
                    actual = screen()
                    actual.feed(payload[:split])
                    actual.feed(payload[split:])
                    self.assertEqual(state(actual), state(expected))

    def test_fragmented_utf8_uses_the_production_incremental_decoder(self):
        payload = 'café 日本 🙂\x1b[2;3Hready'.encode('utf8')
        expected = screen()
        expected.feed(payload.decode('utf8'))
        for split in range(len(payload) + 1):
            decoder = codecs.getincrementaldecoder('utf-8')('replace')
            actual = screen()
            actual.feed(decoder.decode(payload[:split]))
            actual.feed(decoder.decode(payload[split:], final=True))
            self.assertEqual(state(actual), state(expected))

    def test_unterminated_and_malformed_sequences_fail_without_unbounded_retention(self):
        for prefix in ('\x1b[', '\x1b]'):
            value = screen()
            value.feed(prefix + '1' * (MAX_PENDING - 2))
            self.assertEqual(len(value.pending), MAX_PENDING)
            with self.assertRaisesRegex(ValueError, 'exceeds'):
                value.feed('1')
            self.assertLessEqual(len(value.pending), MAX_PENDING)
        with self.assertRaisesRegex(ValueError, 'malformed'):
            screen().feed('\x1b[12\x00not a CSI')

    def test_large_chunks_match_fragmented_input(self):
        payload = ('abcdefghij' * 6554)[:65536] + '\x1b[1;1Hcomplete'
        whole, chunks = screen(), screen()
        whole.feed(payload)
        for at in range(0, len(payload), 137):
            chunks.feed(payload[at:at + 137])
        self.assertEqual(state(whole), state(chunks))
        self.assertEqual(''.join(whole.lines[0][:8]), 'complete')
        self.assertEqual(whole.pending, '')
