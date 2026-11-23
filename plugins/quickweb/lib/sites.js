// efto 内置站点表：一个站点一条记录，图标由管理界面按触发词自绘。
//
// 字段：
//   code   唯一标识（删除内置项时按它记 hidden_sites）
//   cmd    触发词；不带关键词时打开 url，带了关键词且存在 search 时走 search
//   url    主页
//   search 站内搜索地址，{query} 会被替换成编码后的关键词；没有就是纯跳转站
//   group  web / tool / ai / aggregate
'use strict';
module.exports = {
  groups: ['web', 'tool', 'ai', 'aggregate'],
  groupLabels: {
    web: '网页',
    tool: '工具',
    ai: 'AI',
    aggregate: '聚合'
  },
  sites: [
    // ---------- 搜索与内容 ----------
    { code: 'google', cmd: '谷歌搜索', url: 'https://www.google.com', search: 'https://www.google.com/search?q={query}', group: 'web' },
    { code: 'baidu', cmd: '百度搜索', url: 'https://www.baidu.com', search: 'https://www.baidu.com/s?word={query}', group: 'web' },
    { code: 'bing', cmd: '必应搜索', url: 'https://cn.bing.com', search: 'https://cn.bing.com/search?q={query}', group: 'web' },
    { code: 'sogou', cmd: '搜狗搜索', url: 'https://www.sogou.com', search: 'https://www.sogou.com/web?query={query}', group: 'web' },
    { code: 'toutiao', cmd: '头条搜索', url: 'https://www.toutiao.com', search: 'https://so.toutiao.com/search?dvpf=pc&keyword={query}', group: 'web' },
    { code: 'xiaohongshu', cmd: '小红书搜索', url: 'https://www.xiaohongshu.com', search: 'https://www.xiaohongshu.com/search_result?keyword={query}&source=web_explore_feed', group: 'web' },
    { code: 'zhihu', cmd: '知乎', url: 'https://www.zhihu.com', search: 'https://www.zhihu.com/search?type=content&q={query}', group: 'web' },
    { code: 'cnki', cmd: '知网', url: 'https://www.cnki.net', search: 'https://kns.cnki.net/kns8s/defaultresult/index?kw={query}', group: 'web' },
    { code: 'dxy', cmd: '丁香园', url: 'https://www.dxy.cn', search: 'https://drugs.dxy.cn/pc/search?keyword={query}', group: 'web' },
    { code: 'qcc', cmd: '企查查', url: 'https://www.qcc.com', search: 'https://www.qcc.com/web/search?key={query}', group: 'web' },
    { code: 'ths', cmd: '同花顺', url: 'https://www.10jqka.com.cn', search: 'https://www.iwencai.com/unifiedwap/result?tid=stockpick&qs=box_main_ths&w={query}', group: 'web' },
    { code: 'github', cmd: 'Github', url: 'https://github.com', search: 'https://github.com/search?q={query}', group: 'web' },
    { code: 'bilibili', cmd: '哔哩哔哩', url: 'https://www.bilibili.com', search: 'https://search.bilibili.com/all?keyword={query}', group: 'web' },
    { code: 'weather', cmd: '天气', url: 'https://weather.cma.cn', group: 'web' },

    // ---------- 在线工具 ----------
    { code: 'gaoding', cmd: '稿定', url: 'https://www.gaoding.com', search: 'https://www.gaoding.com/templates/{query}?from=input&search_scope=gaoding', group: 'tool' },
    { code: 'huaban', cmd: '花瓣', url: 'https://huaban.com', search: 'https://huaban.com/search?q={query}', group: 'tool' },
    { code: 'islide', cmd: 'iSlide', url: 'https://www.islide.cc', search: 'https://www.islide.cc/ppt/template?keywords={query}', group: 'tool' },
    { code: 'iconfont', cmd: 'iconfont', url: 'https://www.iconfont.cn', search: 'https://www.iconfont.cn/search/index?searchType=icon&q={query}', group: 'tool' },
    { code: 'fonts', cmd: '字体天下', url: 'https://www.fonts.net.cn', search: 'https://www.fonts.net.cn/font-search-result.html?q={query}', group: 'tool' },
    { code: 'smallpdf', cmd: 'Smallpdf', url: 'https://smallpdf.com/cn', group: 'tool' },
    { code: 'removebg', cmd: 'RemoveBg', url: 'https://www.remove.bg/zh', group: 'tool' },
    { code: 'tinify', cmd: 'Tinify', url: 'https://tinify.cn', group: 'tool' },
    { code: 'processon', cmd: 'ProcessOn', url: 'https://www.processon.com/flowcharts', group: 'tool' },
    { code: 'chartcube', cmd: 'ChartCube', url: 'https://chartcube.alipay.com', group: 'tool' },
    { code: 'zhixi', cmd: '知犀', url: 'https://www.zhixi.com', group: 'tool' },
    { code: 'cowtransfer', cmd: '奶牛快传', url: 'https://cowtransfer.com', group: 'tool' },

    // ---------- AI ----------
    { code: 'deepseek', cmd: 'DeepSeek', url: 'https://chat.deepseek.com/', group: 'ai' },
    { code: 'kimi', cmd: 'Kimi', url: 'https://www.kimi.com/', group: 'ai' },
    { code: 'doubao', cmd: '豆包', url: 'https://www.doubao.com/chat/', group: 'ai' },
    { code: 'zai', cmd: 'Z.ai', url: 'https://chat.z.ai/', group: 'ai' },
    { code: 'qianwen', cmd: '千问', url: 'https://www.qianwen.com/', group: 'ai' },
    { code: 'yuanbao', cmd: '元宝', url: 'https://yuanbao.tencent.com/chat', group: 'ai' },
    { code: 'minimax', cmd: 'MiniMax', url: 'https://agent.minimaxi.com/', group: 'ai' },
    { code: 'wenxin', cmd: '文心', url: 'https://chat.baidu.com/', group: 'ai' },
    { code: 'chatgpt', cmd: 'ChatGPT', url: 'https://chatgpt.com/', group: 'ai' },
    { code: 'gemini', cmd: 'Gemini', url: 'https://gemini.google.com/app', group: 'ai' },
    { code: 'claude', cmd: 'Claude', url: 'https://claude.ai/chats', group: 'ai' },

    // ---------- 聚合与资讯 ----------
    { code: 'futunn-cn', cmd: 'A股', url: 'https://www.futunn.com/quote/cn', group: 'aggregate' },
    { code: 'futunn-hk', cmd: '港股', url: 'https://www.futunn.com/quote/hk/stock-list/all-hk-stocks', group: 'aggregate' },
    { code: 'futunn-us', cmd: '美股', url: 'https://www.futunn.com/quote/us/stock-list/all-us-stocks', group: 'aggregate' },
    { code: 'xueqiu', cmd: '雪球', url: 'https://xueqiu.com', search: 'https://xueqiu.com/k?q={query}', group: 'aggregate' },
    { code: '36kr', cmd: '36氪', url: 'https://36kr.com/', group: 'aggregate' },
    { code: 'huxiu', cmd: '虎嗅', url: 'https://m.huxiu.com/', group: 'aggregate' },
    { code: 'tmtpost', cmd: '钛媒体', url: 'https://m.tmtpost.com/', group: 'aggregate' },
    { code: 'techweb', cmd: 'TechWeb', url: 'http://m.techweb.com.cn/', group: 'aggregate' },
    { code: 'geekpark', cmd: '极客公园', url: 'http://www.geekpark.net/', group: 'aggregate' },
    { code: 'unsplash', cmd: 'Unsplash', url: 'https://unsplash.com', search: 'https://unsplash.com/search/photos/{query}', group: 'aggregate' },
    { code: 'pexels', cmd: 'Pexels', url: 'https://www.pexels.com', search: 'https://www.pexels.com/search/{query}/', group: 'aggregate' },
    { code: 'pixabay', cmd: 'Pixabay', url: 'https://pixabay.com/zh/', search: 'https://pixabay.com/zh/photos/{query}/', group: 'aggregate' },
    { code: 'dribbble', cmd: 'Dribbble', url: 'https://dribbble.com', search: 'https://dribbble.com/search?q={query}', group: 'aggregate' },
    { code: 'stocksnap', cmd: 'StockSnap', url: 'https://stocksnap.io', search: 'https://stocksnap.io/search/{query}', group: 'aggregate' },
    { code: 'isorepublic', cmd: 'ISO Republic', url: 'https://isorepublic.com', search: 'https://isorepublic.com/?s={query}', group: 'aggregate' }
  ]
};
