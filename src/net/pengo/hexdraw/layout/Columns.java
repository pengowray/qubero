/*
 * Created on 23/01/2005
 */
package net.pengo.hexdraw.layout;

import java.util.ArrayList;

/**
 * @author Que
 */
public class Columns {
    private ArrayList<SuperSpacer> columns = new ArrayList<SuperSpacer>();
    
    public void addColumn(SuperSpacer unit, int repeats) {
    	Repeater row = new Repeater();
        row.setHorizontal(true);
        row.setContents(unit);
        row.setMaxRepeats(repeats);
        
        Repeater column = new Repeater();
        column.setHorizontal(false);
        column.setContents(row);
        
    	columns.add(column);
    }
    
    public SuperSpacer[] toArray() {
    	return columns.toArray(new SuperSpacer[0]); 
    }
    
    public void clear() {
    	columns.clear();
    }

    
    public GroupSpacer toColumnGroup() {
	   	GroupSpacer page = new GroupSpacer();
		page.setContents(toArray());
		page.setHorizontal(true);
		
		return page;
    }
}
