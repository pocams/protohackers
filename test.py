import codecs
import socket
import re

data = """
80 00 7b 00 08 00 3c
20 04 55 4e 31 58 00 00 00 00
40 00 00 00 0a
"""

# data = """
# 80 00 7b 00 09 00 3c
# 20 04 55 4e 31 58 00 00 00 2d
# """
#
# data = """
# 81 01 00 7b
# """

data = re.sub(r"\s+", "", data)
b = codecs.decode(data, "hex")

sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
sock.connect(("127.0.0.1", 32767))
sock.send(b)

while True:
    r = sock.recv(1024)
    print(r)
