FROM node:18-slim

WORKDIR /app

# The 'ENV' and 'RUN' instructions here can be cached by MemoBuild
ENV NODE_ENV=production

COPY package.json .
RUN npm install

COPY . .

CMD ["node", "index.js"]
