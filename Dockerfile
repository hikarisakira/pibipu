FROM oven/bun:slim

COPY package.json ./
COPY bun.lockb ./
COPY src ./

RUN bun i
CMD [ "bun", "index.ts" ]
