package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;
import java.math.BigInteger;

import net.pengo.hexdraw.original.HexPanel;

public class AddressRenderer extends SeperatorRenderer {

    public AddressRenderer(HexPanel hexpanel, boolean render) {
        super(hexpanel, render);
        setEnabled(render);
    }
    public int renderBytes( Graphics g, long lineNumber,
            byte ba[], int baOffset, int baLength, boolean selecta[],
            int columnWidth ) {
        //// draw address:
        g.setColor(Color.darkGray);
        FontMetrics fm = g.getFontMetrics();
        String addr = BigInteger.valueOf(lineNumber).toString(16)+":       ";
        addr = new String(addr.getBytes(), 0, 8);
        g.drawString(addr, 0, fm.getAscent() );
        return fm.bytesWidth(addr.getBytes(), 0, addr.length()); 
    }
    
    public String toString() {
        return "Address";
    }

}
