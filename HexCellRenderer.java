/*
 * HexCellRenderer.java
 *
 * Created on 30 October 2002, 08:50
 */

import javax.swing.*;
import javax.swing.table.*;
import javax.swing.border.*;
import java.awt.*;

/**
 *
 * @author  Noam Chomsky
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
        Border b = new EmptyBorder(1, left, 1, right); //xxx: nest border
        //Border b = new EmptyBorder(1, right, 1, left); //xxx: nest border
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
