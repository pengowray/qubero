/**
 * ColumnGroupManager.java
 *
 * @author Created by Omnicore CodeGuide
 */
import java.util.*;
import javax.swing.table.*;

//JTableHeader
//TableColumnModel

public class GroupColumnModel extends DefaultTableColumnModel
{
    public static final int ADDR = 1;
    public static final int HEX = 2;
    public static final int ASCII = 3;
    public static final int GREY = 4;

    private ArrayList groupList = new ArrayList();
    private int unitsPerLine;
	
    public GroupColumnModel(int unitsPerLine) {
	super();
	
	this.unitsPerLine = unitsPerLine;
    }
	
    // returns itself (for chaining).
    public GroupColumnModel addColumnGroup(int type) {
	ColumnGroup cg = null;
	
	switch(type) {
	    case ADDR:
		cg = new AddressColumnGroup(unitsPerLine);
		break;
	    case HEX:
		cg = new HexColumnGroup(unitsPerLine);
		break;
	    case ASCII:
		cg = new AsciiColumnGroup(unitsPerLine);
		break;
	    case GREY:
		cg = new GreyColumnGroup(unitsPerLine);
		break;
	}
	
	groupList.add(cg);
	List c = cg.getColumns();
	tableColumns.addAll(c);
	
	return this;
    }
    
    
}

