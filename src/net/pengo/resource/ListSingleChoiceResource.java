/**
 * ListSingleChoiceResource.java
 *
 * A single choice of an item in a list.
 *
 * Perhaps in future a new, dynamic enum type should be created. but this will do for now.
 *
 * fixme: This should also work as a wrapper for the (IntResource) choice , that this can be used as an IntResource
 *
 * fixme: either that, or have an auto-conversion
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import java.util.List;
import net.pengo.pointer.SmartPointer;
import net.pengo.pointer.JavaPointer;
import net.pengo.propertyEditor.PropertyPage;
import net.pengo.propertyEditor.ListSingleChoicePage;

public class ListSingleChoiceResource extends Resource
{
    public final SmartPointer choiceP = new JavaPointer(net.pengo.resource.IntResource.class);
    public final SmartPointer listP = new JavaPointer(net.pengo.resource.ListResource.class);

    public ListSingleChoiceResource(IntResource choice, ListResource list) {
        choiceP.addSink(this);
        choiceP.setName("Choice");
        setChoice(choice);

        listP.addSink(this);
        listP.setName("List");
        setList(list);
	}
	
	public Resource[] getSources()
	{
		return new Resource[] {choiceP, listP};
	}
	
	public IntResource getChoice() {
		return (IntResource)choiceP.evaluate();
	}

	public void setChoice(IntResource choice) {
		choiceP.setValue(choice);
	}
	
	public ListResource getList() {
		return (ListResource)listP.evaluate();
	}

	public void setList(ListResource list) {
		listP.setValue(list);
	}
	
	public PropertyPage getValuePage() {
		return new ListSingleChoicePage(this);
	}
	
    public List getPrimaryPages() {
        List pp = super.getPrimaryPages();
	pp.add(new ListSingleChoicePage(this));
        return pp;
    }	
    
    public String valueDesc() {
	//fixme: maybe ok
        return getList().elementAt(getChoice()).valueDesc();
    }
	
}

