"""Export Terra's YOLO bucket/ball model to the ONNX file the simulator embeds.

    pip install ultralytics onnx onnxslim
    python simulator/tools/export_rover_detector.py ../Terra/models/yolo26n-rover/weights/best.pt

Writes simulator/assets/models/rover_detector.onnx. The export keeps YOLO26's
end-to-end head (nms=False): output [batch, 300, 6], rows x1, y1, x2, y2, score,
class in input pixels. Batch, height and width stay dynamic; the intermediate shape
annotations are dropped because tract cannot unify their symbolic batch with the
fixed input size the simulator sets.
"""
import pathlib
import shutil
import sys

import onnx
from ultralytics import YOLO

weights = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "../Terra/models/yolo26n-rover/weights/best.pt")
out = pathlib.Path(__file__).resolve().parents[1] / "assets" / "models" / "rover_detector.onnx"

exported = YOLO(str(weights)).export(format="onnx", imgsz=640, dynamic=True, simplify=True, opset=17, nms=False)
model = onnx.load(exported)
del model.graph.value_info[:]
onnx.save(model, str(out))
print(f"wrote {out}")
