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

/*
 * HexCellRenderer.java
 *
 * Created on 30 October 2002, 08:50
 */

package net.pengo.app;

import javax.swing.*;
import javax.swing.table.*;
import javax.swing.border.*;
import java.awt.*;

/**
 *
 * @author Peter Halasz
 */
public class HexCellRenderer extends DefaultTableCellRenderer {
    String newVal = ""; // test code
    protected float align;
    protected int unitWidth;

    public HexCellRenderer(float align, int unitWidth) {
	super();
        this.align = align;
        this.unitWidth = unitWidth;
    }

    public Component getTableCellRendererComponent(JTable table, Object value,
                          boolean isSelected, boolean hasFocus, int row, int column) {
        
        Component comp = super.getTableCellRendererComponent(table, value, isSelected, hasFocus, row, column);
        
        int width = table.getColumnModel().getColumn(column).getWidth() - table.getColumnModel().getColumnMargin();
        //int width = getWidth(); // gives 0
        int space = width - unitWidth;
        int left = (int)(align * space);
        int right = (int)((1-align) * space);
        
        //System.out.println(left + "-" + right + " + " + unitWidth + " = " + (left+right+unitWidth) + " = " + width);
        
        
        //Border b = new LineBorder(Color.PINK, 4, true);
        Border b = new EmptyBorder(1, left, 1, right); //FIXME: nest border
        //Border b = new EmptyBorder(1, right, 1, left); //FIXME: nest border
        //System.out.println(left + " " + right);
        setHorizontalAlignment(JLabel.CENTER);
        setBorder(b);
        
        return this; // return comp ? blah whatever.
    }
    
    protected void setValue(Object value) {
	setText((value == null) ? "" : value.toString());
	//setText((value == null) ? ":)" : newVal);
        //Border b = new LineBorder(Color.PINK,2,true); // test
    }
    
}
