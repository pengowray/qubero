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

package net.pengo.hexdraw.lineRepeater;

import java.awt.*;
import javax.swing.JPanel;

class LinePanel extends JPanel {
    protected byte[] bytes; // currently drawing this.
    protected int offset;
    
    /** Creates a new instance of LinePanel */
    public LinePanel() {
        super();
        init();
    }
    public LinePanel(boolean isDoubleBuffered) {
        super(isDoubleBuffered);
        init();
    }
    public LinePanel(LayoutManager layout) {
        super(layout);
        init();
    }
    public LinePanel(LayoutManager layout, boolean isDoubleBuffered) {
        super(layout, isDoubleBuffered);
        init();
    }
    private void init() {
    }

    public void paint(Graphics g, byte[] bytes, int offset) {
        //FIXME: flawed function. sync problems.
        this.bytes = bytes;
        this.offset = offset;
        paint(g);
    }
    
    public void paint(Graphics g) {
        super.paint(g);
        Rectangle r = g.getClipBounds();
        g.setColor(Color.BLUE);
        g.drawLine(r.x, r.y, r.x+r.width, r.y+r.height);
    }
    
    public byte getByte(int offset) {
        return bytes[this.offset + offset];
    }
    
    public UnitInfo getUnitInfo(int offset) {
        return new UnitInfo(getByte(offset), UnitInfo.NORMAL);
    }
        /** calculate size this would be painted at? */
    public Dimension paintSize(byte[] bytes, int offset) {
        return getSize();
    }
}

