"""Fixed OCR regressions exercising the same selectors as the native driver."""
import importlib.util
from pathlib import Path
import unittest

_SPEC = importlib.util.spec_from_file_location(
    'gui_observation', Path(__file__).resolve().parents[1] / 'scripts/parity/gui_observation.py')
observation = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(observation)


def ocr(words):
    """Fixed word tuples: text, left, top, width, height, OCR line id."""
    data = {field: [] for field in
            ('text', 'left', 'top', 'width', 'height', 'block_num', 'par_num', 'line_num')}
    for text, left, top, width, height, line in words:
        for field, value in zip(data, (text, left, top, width, height, 1, 1, line)):
            data[field].append(value)
    return data


class ChooserRegression(unittest.TestCase):
    def test_select_checkbox_is_not_primary_action(self):
        data = ocr([('Select', 18, 6, 72, 24, 1), ('all', 99, 6, 30, 24, 1),
                    ('Cancel', 330, 111, 96, 27, 2), ('Select', 660, 111, 99, 27, 2)])
        lines = observation.ocr_lines(data, (580, 690, 900, 750), 3)
        self.assertEqual(observation.chooser_primary(lines), (816.5, 731.5))
        self.assertIsNone(observation.chooser_primary(lines, save=True))

    def test_open_and_save_require_cancel_in_same_frame(self):
        for text, save in (('Open', False), ('Save', True)):
            with self.subTest(text=text):
                data = ocr([(text, 40, 8, 30, 12, 1),
                            ('Cancel', 120, 30, 40, 12, 2), (text, 220, 30, 30, 12, 2)])
                lines = observation.ocr_lines(data, (0, 0, 320, 60), 1)
                self.assertEqual(observation.chooser_primary(lines, save), (235, 36))
                self.assertIsNone(observation.chooser_primary([lines[0], [lines[1][1]]], save))

    def test_ambiguous_primary_fails(self):
        lines = [[('Cancel', 10, 20, 40, 12), ('Select', 70, 20, 40, 12),
                  ('Open', 150, 20, 30, 12)]]
        with self.assertRaisesRegex(ValueError, 'Ambiguous'):
            observation.chooser_primary(lines)


class ExportRegression(unittest.TestCase):
    def test_header_only_defines_crop_for_separate_button_observation(self):
        data = ocr([('Export', 30, 60, 90, 30, 1), ('options', 135, 60, 105, 30, 1),
                    ('Exported', 30, 240, 120, 30, 2)])
        lines = observation.ocr_lines(data, (700, 0, 910, 700), 3)
        self.assertEqual(observation.export_button_boxes(lines, (1120, 800)), [(700, 32, 790, 50)])
        self.assertIsNone(observation.export_button_point([lines[0]]))
        button = ocr([('Export', 18, 9, 90, 27, 1)])
        button_lines = observation.ocr_lines(button, (700, 32, 790, 50), 3)
        self.assertEqual(observation.export_button_point(button_lines), (721, 39.5))

    def test_already_selected_16_bit_does_not_open_menu(self):
        data = ocr([('16', 30, 60, 24, 24, 1), ('bit', 60, 60, 30, 24, 1),
                    ('Bitdepth', 180, 60, 120, 24, 1)])
        lines = observation.ocr_lines(data, (100, 100, 400, 300), 3)
        self.assertIsNone(observation.selected_value(lines, (100, 100, 400, 300), 'Bit depth', '16 bit'))
        current = observation.selected_value(lines, (100, 100, 400, 300), 'Bit depth', '8 bit')
        self.assertEqual(current, ((119, 124), 160, 124))

    def test_contrast_passes_do_not_build_a_false_16_bit_span(self):
        first = ocr([('16', 20, 20, 15, 12, 1), ('Bitdepth', 130, 20, 70, 12, 1)])
        second = ocr([('8', 20, 20, 10, 12, 1), ('bit', 35, 20, 18, 12, 1),
                      ('Bitdepth', 130, 20, 70, 12, 1)])
        lines = observation.ocr_lines(first, (100, 100, 400, 300), 1)
        lines += observation.ocr_lines(second, (100, 100, 400, 300), 1)
        self.assertIsNone(observation.matched_span(lines[0], '16 bit'))
        self.assertEqual(observation.selected_value(lines, (100, 100, 400, 300), 'Bit depth', '16 bit'),
                         ((134, 126), 230, 126))

    def test_conflicting_selected_values_fail(self):
        lines = [[('16bit', 110, 120, 40, 12), ('Bitdepth', 230, 120, 70, 12)],
                 [('8bit', 110, 120, 40, 12), ('Bitdepth', 230, 120, 70, 12)]]
        with self.assertRaisesRegex(ValueError, 'Ambiguous'):
            observation.selected_value(lines, (100, 100, 400, 300), 'Bit depth', '16 bit')

    def test_alternative_passes_keep_selected_bit_depth_in_one_row(self):
        first = ocr([('16bit', 30, 60, 60, 24, 1), ('Bitdepth', 180, 60, 120, 24, 1)])
        second = ocr([('16', 33, 63, 24, 24, 1), ('bit', 63, 63, 30, 24, 1),
                      ('Bitdepth', 183, 63, 120, 24, 1)])
        lines = observation.ocr_lines(first, (100, 100, 400, 300), 3)
        lines += observation.ocr_lines(second, (100, 100, 400, 300), 3)
        self.assertIsNone(observation.selected_value(lines, (100, 100, 400, 300), 'Bit depth', '16 bit'))


    def test_menu_ignores_current_value_and_words_outside_modal(self):
        lines = [[('16bit', 110, 110, 40, 12)], [('16bit', 110, 150, 40, 12)],
                 [('16bit', 30, 150, 40, 12)], [('16bit', 110, 310, 40, 12)]]
        self.assertEqual(observation.menu_option(lines, (100, 100, 400, 300), '16 bit', 230, 116),
                         (130, 156))
        with self.assertRaisesRegex(ValueError, 'Ambiguous'):
            observation.menu_option(lines + [[('16bit', 110, 190, 40, 12)]],
                                    (100, 100, 400, 300), '16 bit', 230, 116)

    def test_submit_rejects_heading_and_uses_same_frame_cancel(self):
        lines = [[('Export', 110, 110, 40, 12), ('options', 160, 110, 40, 12)],
                 [('Export', 110, 260, 40, 12), ('Cancel', 230, 260, 40, 12)]]
        self.assertEqual(observation.modal_submit(lines, (100, 100, 400, 300), 700),
                         ((130, 266), (250, 266)))
        self.assertEqual(observation.modal_submit([lines[0]], (100, 100, 400, 300), 700), (None, None))
        row = [[('(Export)', 112, 260, 40, 12)]]
        self.assertEqual(observation.submit_row(row, (250, 266)), (132, 266))
        self.assertIsNone(observation.submit_row(row, (250, 210)))

    def test_separate_crop_geometry_is_not_reused_between_passes(self):
        data = ocr([('Export', 12, 18, 60, 24, 1)])
        self.assertEqual(observation.export_button_point(observation.ocr_lines(data, (700, 32, 790, 50), 3)),
                         (714, 42))
        self.assertEqual(observation.export_button_point(observation.ocr_lines(data, (100, 200, 200, 260), 2)),
                         (121, 215))

    def test_bad_ocr_columns_and_out_of_crop_boxes_fail(self):
        data = ocr([('Export', 12, 18, 60, 24, 1)])
        data['width'] = []
        with self.assertRaisesRegex(ValueError, 'Malformed'):
            observation.ocr_lines(data, (0, 0, 100, 100), 1)
        for left, width in ((-1, 20), (90, 20), (10, float('nan'))):
            with self.subTest(left=left, width=width), self.assertRaises(ValueError):
                observation.ocr_lines(ocr([('Export', left, 20, width, 12, 1)]), (0, 0, 100, 100), 1)

    def test_heading_crop_must_stay_inside_frame(self):
        with self.assertRaisesRegex(ValueError, 'outside frame'):
            observation.export_button_boxes([[('Exportoptions', 2, 20, 80, 12)]], (1120, 800))


if __name__ == '__main__':
    unittest.main()
