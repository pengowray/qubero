package net.pengo.splash;

import java.awt.Font;
import java.awt.FontMetrics;
import java.awt.Graphics;
import java.io.InputStream;
import java.net.URL;
import java.util.HashMap;
import java.util.Iterator;
import java.util.Map;
import java.util.Set;

/** because it can be difficult to find font metric properties in a hurry.
  Especially when you dont have a Graphics object*/
public class FontMetricsCache {
    
    protected static FontMetricsCache singleton;

    protected Map<Font,FontMetrics> fontMap; // font -> FontMetrics
    
    protected Map<SizedFont,Font> fontNameMap; // SizedFont -> Font
    protected Map<SimplySizedFont,Font> fontSimpleSizeMap;
    
    protected boolean allFound = true; // have all metrics been located?
    
    public static FontMetricsCache singleton() {
        if (singleton == null)
            singleton = new FontMetricsCache();
        
        return singleton;
    }
    
    // use singleton() instead of this constructor.
    private FontMetricsCache() {
        fontMap = new HashMap<Font,FontMetrics>();
        fontNameMap = new HashMap<SizedFont,Font>();
        fontSimpleSizeMap = new HashMap<SimplySizedFont,Font>();
        
        allFound = true;
        
        //***********
        // put registrations here:
        
        registerFontGroup("hex", new Font("Monospaced", Font.BOLD, 11) );
        
        registerFontGroup("dos", new Font("Monospaced", Font.PLAIN, 11) );

        //fixme: should try: Lucida Sans Unicode, arial unicode ms, Microsoft Sans Serif, SansSerif
        registerFontGroup("unicode", new Font("Lucida Sans Unicode", Font.BOLD, 11) );
        
        registerFontGroup("alien", ecclemonyFont()); // PreEcclenony Regular, or Futurama Alien Alphabet One
        
        //registerFontToFind("futurama", new Font("Futurama Alien Alphabet One", Font.PLAIN, 12) );
        
        //***********
        
    }
    
    private void registerFontGroup(String name, Font basefont) {
    	
    	for (SimpleSize ss : SimpleSize.allSizes() ) {
    		Font derived = basefont.deriveFont(ss.toFontSize());
    		
    		SizedFont sfont = new SizedFont(name,ss.toFontSize());
            fontNameMap.put(sfont, derived);

    		SimplySizedFont ssfont = new SimplySizedFont(name,ss);
            fontSimpleSizeMap.put(ssfont, derived);
            
//            System.out.println("added simplysizedfont:" + ssfont +", font:" + derived);
//            System.out.println("retrieving(1): " + fontSimpleSizeMap.get(ssfont));
//            System.out.println("retrieving(2): " + fontSimpleSizeMap.get(new SimplySizedFont(name,simpleSize)));
            
            if (fontMap.get(derived) == null) {
	            fontMap.put(derived, null);
	            allFound = false;
            }
            
    	}
    }
    
    private Font ecclemonyFont() {
        //String futuramaFile = "net/pengo/noncode/fr-fal1.ttf";
        //String futuramaFile = "net/pengo/noncode/fr-title.ttf";
        //String futuramaFile = "net/pengo/noncode/Bin0011.ttf";
        String futuramaFile = "net/pengo/noncode/PrEcclem.ttf"; // PreEcclenony Regular
        //String futuramaFile = "net/pengo/noncode/falsepos.ttf"; // False Positive non commercial use
        //String futuramaFile = "net/pengo/noncode/reasonsh.ttf"; // non commercial use
        //String futuramaFile = "net/pengo/noncode/reason.ttf"; // non commercial use
        //String futuramaFile = "net/pengo/noncode/sequence.ttf"; // non commercial use
        //String futuramaFile = "net/pengo/noncode/zurklezo.ttf";
        //String futuramaFontName = "Futurama Alien Alphabet One"; // hmm.. not needed
        int size = 12;
        
        try {
            URL url = ClassLoader.getSystemResource(futuramaFile);
            InputStream futureStream = url.openStream();
            Font futurama = Font.createFont(Font.TRUETYPE_FONT, futureStream);
            futureStream.close();
            Font futurama12 = futurama.deriveFont((float)size);
            //registerFontToFind("alien", futurama12);
            return futurama12;
        } catch (Exception e) {
            //IOException or FontFormatException 
            //fall back to monospaced.
            System.out.println("Alien font not found.");
            return new Font("Monospaced", Font.PLAIN, size);
            //registerFontToFind("alien", new Font("Monospaced", Font.PLAIN, size));
        }
    }

    /* generously lend this a graphics object so the empty
     items in the cache can be filled. */
    public void lendGraphics(Graphics g) {
        if (allFound)
            return;
        
        Set set = fontMap.keySet();
        for (Iterator i = set.iterator(); i.hasNext(); ) {
            Font key = (Font)i.next();
            if (fontMap.get(key) == null) {
                fontMap.put(key, g.getFontMetrics(key));
            }
        }
        allFound = true;
        //System.out.println("all found:" + allFound);
    }
    
//    /** register a font you will later need the font metrics for.
//      just add a call to this to the private constructor above. */
//    protected void registerFontToFind(SizedFont name, Font f) {
//        fontNameMap.put(name, f);
//        
//        if (fontMap.get(f) != null)
//            return;
//        
//        fontMap.put(f, null);
//        allFound = false;
//        //System.out.println("added \"" + name + "\" -- " + f);
//    }

    /** depreciated method!*/
    public FontMetrics getFontMetrics(String name) {
    	return getFontMetrics(new SimplySizedFont(name, SimpleSize.defaultSimpleSize()));
    }

    /** depreciated method!*/
    public Font getFont(String name) {
    	return getFont(new SimplySizedFont(name, SimpleSize.defaultSimpleSize()));
    }
    
  	public FontMetrics getFontMetrics(SizedFont name) {
        //FIXME: block until font comes in?
        //System.out.println("getting metrics" + name);
        return fontMap.get(getFont(name));
    }

    public FontMetrics getFontMetrics(SimplySizedFont name) {
        //FIXME: block until font comes in?
        //System.out.println("getting metrics" + name);
        return fontMap.get(getFont(name));
    }
    
    public FontMetrics getFontMetrics(Font f) {
        //System.out.println("getting metrics" + f);
        FontMetrics fm = (FontMetrics)fontMap.get(f);
        if (fm==null) {
            System.out.println("this font never registered: " + f);
        }
            
        return fm;
    }
    
    public Font getFont(SizedFont name) {
        //System.out.println("getting font" + name);
        return fontNameMap.get(name);
    }

    public Font getFont(SimplySizedFont name) {
    	return fontSimpleSizeMap.get(name);
    }

}

