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

/**
 * HexUnit.java
 *
 * @author  Peter Halasz
 */

package net.pengo.hexdraw.lineRepeater;

import java.awt.*;
import net.pengo.splash.FontMetricsCache;

class HexUnit extends Component {
    protected FontMetrics fm;
    protected Dimension prefSize;
    protected int offset; // used when asking parent for a unit.
    
    //FIXME: getHowManyPrevAndForwardBytesNeededToDraw ?
    
    public HexUnit(int offset){
        super();
        this.offset = offset;
        setFont(FontMetricsCache.singleton().getFont("hex"));
        fm = FontMetricsCache.singleton().getFontMetrics("hex");
    }
    
    public void paint(Graphics g) {

        Container con = getParent();
        if (con instanceof LinePanel) {
            LinePanel linpan = (LinePanel)con;
            paint(g, linpan.getUnitInfo(offset));
        }
        // else do nothing
    
        ////
        Rectangle r = g.getClipBounds();
        g.setColor(Color.RED);
        g.drawLine(r.x, r.y, r.x+r.width, r.y+r.height);
        System.out.println("drawing " + offset);
        ////
    }

    public void paint(Graphics g, UnitInfo unit) {
        
        if (unit.getState() == UnitInfo.EMPTY)
            return;
        
        String str = unit.hexString();
        g.drawString(str, 0, fm.getAscent());
        System.out.println("drawing " + str); // xxx
    }
    
    public Dimension getPreferredSize() {
        if (prefSize == null) {
            prefSize = new Dimension(fm.getMaxAdvance() * 2, fm.getHeight());
        }
        return prefSize;
    }
    
    public Dimension getMinimumSize() {
        return getPreferredSize();
    }
    
    public Dimension getMaximumSize() {
        // allow this to be bigger maybe?
        return getPreferredSize();
    }
    
}

