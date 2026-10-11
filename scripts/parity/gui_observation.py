"""Deterministic native-driver OCR selection, independent of desktop dependencies."""
import math
import re


def normalized(text):
    return ''.join(re.findall(r'[a-z0-9]+', text.lower()))


def ocr_lines(data, box, scale):
    """Map one OCR pass to frame coordinates; never merge alternative passes."""
    if scale <= 0 or len(box) != 4 or box[2] <= box[0] or box[3] <= box[1]:
        raise ValueError('Invalid OCR crop or scale')
    fields = ('text', 'left', 'top', 'width', 'height', 'block_num', 'par_num', 'line_num')
    if any(field not in data or len(data[field]) != len(data['text']) for field in fields):
        raise ValueError('Malformed OCR word columns')
    grouped = {}
    for i, text in enumerate(data['text']):
        if not isinstance(text, str):
            raise ValueError('Malformed OCR text')
        if not text.strip():
            continue
        geometry = tuple(data[field][i] for field in ('left', 'top', 'width', 'height'))
        if any(not isinstance(value, (int, float)) or not math.isfinite(value)
               for value in geometry) or min(geometry) < 0 or min(geometry[2:]) <= 0:
            raise ValueError('Malformed OCR word geometry')
        left, top, width, height = geometry
        if left + width > (box[2] - box[0]) * scale or top + height > (box[3] - box[1]) * scale:
            raise ValueError('OCR word lies outside its crop')
        if text.casefold() == 'bitdepth':
            text = 'Bit depth'
        key = tuple(data[field][i] for field in ('block_num', 'par_num', 'line_num'))
        grouped.setdefault(key, []).append((text, box[0] + left / scale,
            box[1] + top / scale, width / scale, height / scale))
    return list(grouped.values())


def matched_span(line, label):
    wanted = normalized(label)
    for start in range(len(line)):
        text = ''
        for end in range(start, len(line)):
            text += normalized(line[end][0])
            if text == wanted or (wanted == '16bit' and text == '16bir'):
                return line[start:end + 1]
            if len(text) >= len(wanted):
                break
    return None


def points(lines, label):
    result = []
    for line in lines:
        for start in range(len(line)):
            span = matched_span(line[start:], label)
            if span and span[0] is line[start]:
                result.append((span[0][1] + sum(word[3] for word in span) / 2,
                               span[0][2] + span[0][4] / 2))
    return result


def unique_point(candidates, label):
    # Contrast passes may observe the same glyphs with slightly different boxes.
    if not candidates:
        return None
    first = candidates[0]
    if any(abs(x - first[0]) > 10 or abs(y - first[1]) > 10 for x, y in candidates[1:]):
        raise ValueError(f'Ambiguous {label}: {candidates}')
    return first


def chooser_primary(lines, save=False):
    words = [word for line in lines for word in line]
    cancels = [word for word in words if word[0].strip().casefold() == 'cancel']
    if len(cancels) > 1:
        raise ValueError('Ambiguous native chooser Cancel target')
    expected = {'save', 'ok'} if save else {'open', 'select', 'ok'}
    candidates = []
    if len(cancels) == 1:
        cancel = cancels[0]
        cancel_y = cancel[2] + cancel[4] / 2
        for word in words:
            y = word[2] + word[4] / 2
            if (word[0].strip().casefold() in expected
                    and abs(y - cancel_y) <= max(word[4], cancel[4]) / 2
                    and word[1] > cancel[1] + cancel[3]):
                candidates.append((word[1] + word[3] / 2, y))
    if len(candidates) > 1:
        raise ValueError(f'Ambiguous native chooser primary action: {candidates}')
    return candidates[0] if candidates else None


def export_button_boxes(lines, frame):
    boxes = []
    for line in lines:
        header = [word for word in line if normalized(word[0]) in ('export', 'options', 'exportoptions')]
        if ''.join(normalized(word[0]) for word in header) != 'exportoptions':
            continue
        height = max(word[4] for word in header)
        bottom = max(word[2] + word[4] for word in header)
        box = (int(min(word[1] for word in header) - height), int(bottom + height / 4),
               int(max(word[1] + word[3] for word in header) + height),
               min(frame[1] - 100, int(bottom + height * 2)))
        if box[3] <= box[1]:
            continue
        if box[0] < 0 or box[1] < 0 or box[2] > frame[0]:
            raise ValueError(f'Export button crop outside frame: {box}')
        boxes.append(box)
    if len(boxes) > 1:
        raise ValueError(f'Ambiguous Export options heading: {boxes}')
    return boxes


def export_button_point(lines):
    words = [word for line in lines for word in line]
    if len(words) == 1 and normalized(words[0][0]) == 'export':
        word = words[0]
        return word[1] + word[3] / 2, word[2] + word[4] / 2
    return None


def selected_value(lines, box, label, option):
    rows = []
    for line in lines:
        span = matched_span(line, label)
        if span is None and len(label.split()) > 1:
            span = matched_span(line, label.split()[-1])
        if span is None:
            continue
        label_left = min(word[1] for word in span)
        y = span[0][2] + span[0][4] / 2
        values = [word for word in line
                  if box[0] <= word[1] and word[1] + word[3] < label_left
                  and box[1] <= word[2] and word[2] + word[4] <= box[3]
                  and abs(word[2] + word[4] / 2 - y) < 10]
        if values:
            rows.append((values, label_left, y))
    if not rows:
        raise ValueError(f'Export options selected value is not visible: {label}')
    choices = ('JPEG', 'PNG', 'TIFF', 'EXR') if label == 'Format' else ('8 bit', '16 bit', '32 bit')
    recognized = [(choice, point, left, y) for values, left, y in rows
                  for choice in choices for point in points([values], choice)]
    if not recognized:
        raise ValueError(f'Export options selected value is not recognized: {label}')
    if len({choice for choice, _, _, _ in recognized}) != 1:
        raise ValueError(f'Ambiguous export selected value: {label}')
    unique_point([item[1] for item in recognized], label)
    if any(points([values], option) for values, _, _ in rows):
        return None
    _, point, left, y = recognized[0]
    return point, left, y


def menu_option(lines, box, option, label_left, row_y):
    candidates = [(x, y) for x, y in points(lines, option)
                  if box[0] <= x < min(label_left, box[2]) and row_y + 5 < y <= box[3]]
    return unique_point(candidates, f'export option {option}')


def modal_submit(lines, box, right_control_x):
    cancels = [(x, y) for x, y in points(lines, 'Cancel')
               if box[0] <= x < min(right_control_x, box[2]) and box[1] <= y <= box[3]]
    candidates = [(x, y) for x, y in points(lines, 'Export')
                  if box[0] <= x <= box[2] and box[1] <= y <= box[3]
                  and any(x < cx and abs(y - cy) < 12 for cx, cy in cancels)]
    return unique_point(candidates, 'export submit'), unique_point(cancels, 'export Cancel')


def submit_row(lines, cancel):
    """Use the Cancel observed in this frame to bound the independent row pass."""
    cx, cy = cancel
    candidates = [(word[1] + word[3] / 2, word[2] + word[4] / 2)
                  for line in lines for word in line
                  if re.findall(r'[a-z0-9]+', word[0].lower()) == ['export']
                  and word[1] + word[3] / 2 < cx
                  and abs(word[2] + word[4] / 2 - cy) < 12]
    return unique_point(candidates, 'export submit row')


def record_observation(root, records, action, image, box, passes, chosen=None, cropped=False):
    """Retain the exact crop and word geometry used for a native action."""
    path = root / f'{len(records):02d}-{action}-crop.png'
    (image if cropped else image.crop(box)).save(path)
    records.append({'surface': action, 'screenshot': path.name, 'crop': list(box),
                    'ocr_passes': passes, 'chosen_coordinates': chosen})
    return records[-1]
