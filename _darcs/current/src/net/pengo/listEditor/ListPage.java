/**
 * listPage.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.listEditor;
import net.pengo.propertyEditor.EditablePage;
import net.pengo.propertyEditor.PropertiesForm;
import net.pengo.resource.ListResource;
import javax.swing.JTable;
import javax.swing.JLabel;
import javax.swing.table.TableModel;
import javax.swing.table.DefaultTableModel;
import net.pengo.propertyEditor.MultiPage;
import net.pengo.resource.IntPrimativeResource;
import net.pengo.propertyEditor.SummaryPage;
import net.pengo.propertyEditor.TextOnlyPage;
import net.pengo.propertyEditor.PropertyPage;
import javax.swing.tree.DefaultMutableTreeNode;
import java.util.Iterator;
import net.pengo.resource.Resource;
import java.util.ArrayList;
import java.util.List;

public class ListPage extends MultiPage
{
	private ListResource list;
	
	public ListPage(PropertiesForm form, ListResource listResource)
	{
		super(form, "list or array");
		
		this.list = listResource;
		
			List subpageList = new ArrayList();
			
		//TextOnlyPage Page1 = new TextOnlyPage("item1", "" + list.elementAt(new IntPrimativeResource(0)));
		//TextOnlyPage Page2 = new TextOnlyPage("item2", "" + list.elementAt(new IntPrimativeResource(1)));
		
		for (Iterator i = list.iterator(); i.hasNext(); ) {
			Resource r = (Resource)i.next();

			//fixme: evaluate, really? shouldn't this be an option or smoething?
			PropertyPage pp = r.evaluate().getValuePage();
			if (pp != null) {
				subpageList.add(pp);
			}
		}
		
		setPages((PropertyPage[])subpageList.toArray(new PropertyPage[0]));
		
		/*
		JTable table = new JTable();
		DefaultTableModel dataModel = new DefaultTableModel();
		dataModel.addColumn("#",new Integer[]{new Integer(1),new Integer(2),new Integer(3)});
		dataModel.addColumn("#",new String[]{"abc","def","ghi"});
		table.setModel(dataModel);
		add(table);
		 */
		//add(new JLabel("test"));
		
		
	}
	
	protected void saveOp()
	{
		// TODO
	}
	
	public void buildOp()
	{
		// TODO
	}
	
	public String toString()
	{
		return "List/Array editor";
	}
	
	
}

