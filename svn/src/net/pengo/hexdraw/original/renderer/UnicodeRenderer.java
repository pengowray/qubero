/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.Font;
import java.awt.FontMetrics;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.io.UnsupportedEncodingException;

import net.pengo.hexdraw.original.Place;
import net.pengo.splash.FontMetricsCache;

public class UnicodeRenderer extends SeperatorRenderer {
    private Font unicodeFont;
    private FontMetrics ufm;
    private Font futurama;
    
    private boolean hideHighAscii; // hide ascii > 127 and control characters (0
    
    public UnicodeRenderer(FontMetrics fm, int columnCount, boolean render) {
        super(fm, columnCount, render);
        //fixme: move unicode font to cache thing
        unicodeFont = FontMetricsCache.singleton().getFont("unicode");
        ufm = FontMetricsCache.singleton().getFontMetrics("unicode");
        futurama = FontMetricsCache.singleton().getFont("alien");
    }
    
    private String byte2ascii(byte b)  {
        int unsigned = (((int)b) & 0xff);
        String hex = Integer.toString(unsigned, 16);

        if (hex.length() == 1) {
                hex = "0"+hex;
        }
    
        return hex;
        
    }
    
    public void renderBytes( Graphics g1, long lineNumber,
            byte ba[], int baOffset, int baLength, boolean selecta[],
            Place cursor){
        
        Graphics2D g = (Graphics2D)g1;
        
        // anti aliasing
        
        RenderingHints renderHints =
            new RenderingHints(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
        renderHints.put(RenderingHints.KEY_RENDERING, RenderingHints.VALUE_RENDER_QUALITY);
        RenderingHints savedRenderHints = g.getRenderingHints();
        g.setRenderingHints(renderHints);
        
        Font saved = g.getFont();
        
        //g.setFont(unicodeFont);
        g.setFont(futurama);
        
        
        //// draw ascii:
        
        int charsWidth = ufm.charWidth('W');
        int charsHeight = fm.getHeight();
        //System.out.println(futurama);
        
        //System.out.println(futurama.getFamily() + "display up to # " + futurama.canDisplayUpTo("234567890ABCDEFGHIJK1"));
        for ( int i=0; i < baLength; i++ ) {
            if (selecta[i])  {
                //g.setColor( GUI.selectionColor() );
                g.setColor( new Color(170,170,255) ); // FIXME: cache
                g.fillRect( i * charsWidth * 2, 0, charsWidth * 2, charsHeight);  //FIXME: precision loss!
            }
            g.setColor(Color.black);
            //g.setColor(Color.darkGray);
            String ascii = byte2ascii(ba[baOffset+i]);
            
            //force monospacing
            char l = ascii.charAt(0);
            char r = ascii.charAt(1);
            g.drawString( l+"", i * charsWidth * 2, fm.getAscent());
            g.drawString( r+"", i * charsWidth * 2 + charsWidth, fm.getAscent());
            
            //g.drawString( ascii, i * charsWidth * 2, fm.getAscent());
        }
        //g.drawString( "hello 1234567890abcd", 17 * charsWidth, fm.getAscent()); //FIXME: precision loss!
        
        g.setRenderingHints(savedRenderHints);
        g.setFont(saved);
        
        //return columnCount * charsWidth;
    }
    
    public int getWidth() {
        //return fm.charWidth('W')*columnCount;
        return ufm.charWidth('W')*columnCount*2;
    }
    
    public long whereAmI(int x, int y, long lineAddress){
        return lineAddress + (x / (ufm.charWidth('W')*2));
    }
    
    public String toString() {
        
        return "Alien hex";
    }
    
}
