/**
 * ListAddressedResource.java
 *
 * Simple.. All items the same size (x bytes)
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import net.pengo.pointer.SmartPointer;
import net.pengo.pointer.JavaPointer;
import net.pengo.pointer.QFunction;
import net.pengo.selection.LongListSelectionModel;

public class ListAddressedResource extends ListResource {
    
    public final SmartPointer selResP = new JavaPointer(net.pengo.resource.SelectionResource.class);
    public final SmartPointer itemLengthP = new JavaPointer(net.pengo.resource.IntResource.class);
    public final SmartPointer constructorP = new JavaPointer(net.pengo.pointer.QFunction.class);
    
    public final SmartPointer count = new JavaPointer(net.pengo.resource.IntResource.class); //fixme: derrived / dependent
    
    public ListAddressedResource(SelectionResource selRes) {
	//super(); // works too
	super(new JavaType(IntAddressedResource.class));
	
	constructor(selRes);
    }
    
    public ListAddressedResource(SelectionResource selRes, TypeResource type) {
	//super(new JavaType(IntAddressedResource.class));
	super(type);
	
	constructor(selRes);
    }
    
    private void constructor(SelectionResource selRes) {

	System.out.println("superfinished.. make selection ");
	
	selResP.addSink(this);
	selResP.setName("Selection");
	setSelectionResource(selRes);
	
	itemLengthP.addSink(this);
	itemLengthP.setName("Item length");
	itemLengthP.setValue(new IntPrimativeResource(4));
	
	count.addSink(this);
	count.setName("Item count");

	//fixme: need a selection length resource
	//fixme: need a way to edit the division resource
	//setCount(new IntPrimativeResource(selRes.getSelection().getSelectionIndexes().length/4));
	//fixme: fix IntegerDivisionResource so can do this in one step, or less messy... new IntPrimativeResource(4) is not used.
	IntegerDivisionResource div = new IntegerDivisionResource(new IntPrimativeResource(selRes.getSelection().getSelectionIndexes().length), new IntPrimativeResource(4));
	div.denominatorP.setValue(itemLengthP);
	
	//System.out.println("setting count value to div's value");
	
	count.setValue(div.valueP);
	
	//System.out.println("constructor bit");

	/* constructor for building objects from the array */
	constructorP.addSink(this);
	constructorP.setName("Constructor");
	System.out.println("assigning a constructor..");
	QFunction[] qfs = getType().getConstructorsForSelections();
	if (qfs.length > 0) {
	    QFunction cons = qfs[0];
	    System.out.println("constructor: " + cons);
	    constructorP.setValue(cons); // FIXME!!
	} else {
	    System.out.println("no constructors for:" + getType());
	}
    }
    
    public SelectionResource getSelectionResource() {
	return (SelectionResource)selResP.evaluate();
    }
    
    public void setSelectionResource(SelectionResource selRes) {
	selResP.setValue(selRes);
    }
    
    public Resource[] getSources() {
	return new Resource[] { type, selResP, itemLengthP, count };
    }
    
    public Resource elementAt(IntResource index) {
	//return array[index.getValue().intValue()];
	QFunction cons = getConstructor();
	long il = getItemLength().toLong();
	LongListSelectionModel elementSelection = getSelection().getSelection().slice(il*index.toLong(), il);
	DefaultSelectionResource selResource = new DefaultSelectionResource(getSelection().getOpenFile(), elementSelection);
	
	SmartPointer ret = cons.invoke(null, new Resource[] { selResource });
	
	return ret;
    }
    
    public void setElement(IntResource index, Resource value) {
	//array[index.getValue().intValue()] = value;
    }
    
    public QFunction getConstructor() {
	return (QFunction)constructorP.evaluate();
    }
    
    public SelectionResource getSelection() {
	return (SelectionResource)selResP.evaluate();
    }
    
    public IntResource getItemLength() {
	return (IntResource)itemLengthP.evaluate();
    }
    
    
    public void setCount(IntResource count) {
	this.count.setValue(count);
    }
    
    public IntResource getCount() {
	return (IntResource)count.evaluate();
    }

    public void setValue(Resource[] array) {
	setCount(new IntPrimativeResource(array.length));
	for (int c=0; c<array.length; c++) {
	    setElement(new IntPrimativeResource(c), array[c]);
	}
    }
    
    public String valueDesc() {
	return "(list)";
    }
}

