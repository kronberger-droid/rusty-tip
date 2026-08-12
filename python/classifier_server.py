"""Reference classifier sidecar for rusty-tip's `HttpClassifier`.

Wire contract (see `src/classifier/http.rs`):

- `GET /info` returns the model identity, resolved once at connect time.
- `POST /classify` receives the payload as npy bytes in the body and its
  JSON metadata (channel, shape, direction, physical geometry) in the
  `X-Payload-Meta` header, and returns the verdict as the JSON body.

Replace the body of `classify` with a real model; everything else is the
whole protocol. Run with:

    uvicorn classifier_server:app --port 8000
"""

import io
import json

import numpy as np
from fastapi import FastAPI, Request

app = FastAPI()


@app.get("/info")
def info():
    return {"name": "mean_threshold", "version": "0.1.0"}


@app.post("/classify")
async def classify(request: Request):
    frame = np.load(io.BytesIO(await request.body()))
    meta = json.loads(request.headers["x-payload-meta"])
    return {
        "usable": bool(np.isfinite(frame).all()),
        "score": float(frame.std()),
        "channel": meta["channel_name"],
    }
