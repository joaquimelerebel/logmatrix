import time
import logging

logging.basicConfig(level=logging.DEBUG)

with open("log.log", "r") as f:
    for line in f:
        lne = line.strip()
        logging.debug(f"{line}")
        time.sleep(0.5)
time.sleep(10)
